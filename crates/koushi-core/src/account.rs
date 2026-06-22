//! AccountActor: handles login, restore, logout, account switch, and owns the
//! SyncActor child.
//!
//! Owns the `MatrixClientSession` handle and the `StoreActor` (which owns
//! the single `CredentialStoreBackend` used for both unlock secrets and
//! session persistence). Internal outcomes are projected via the runtime's
//! action channel (AppAction::LoginSucceeded etc.) so AppState / StateChanged
//! remain reducer-driven. Domain events (AccountEvent::LoggedIn etc.) plus
//! OperationFailed are emitted on the CoreEvent stream.
//!
//! Account store bootstrap invariant (overview.md, Runtime Model): per-account
//! store paths derive from homeserver|user|device, so the device id is unknown
//! until the password exchange completes. First login runs on a storeless
//! client that never syncs or initializes encryption; immediately after login
//! the session is persisted and restored into the per-account encrypted store,
//! and only the store-backed session may start sync or E2EE traffic. The
//! fail-closed local-encryption rule applies to the store creation step: if it
//! fails, the storeless session is NOT kept as a fallback.
//!
//! SwitchAccount (overview.md): ordered shutdown of the current account
//! runtime WITHOUT clearing credentials or stores, followed by a store-backed
//! restore of the target account. Shutdown order: timelines → search → sync
//! (phases 4, 5, 6 add their children; Phase 3 adds sync).
//!
//! Shutdown order (overview.md Async rules 11/12, rule 12 step 4):
//!   stop timeline subscriptions → stop search → stop sync → drop SDK handles.
//! SDK handles dropped inside the Tokio runtime context.

use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::Arc,
    time::Duration,
};

use futures_util::StreamExt;
use koushi_key::{SessionKeyId, StoredMatrixSession};
use koushi_sdk::{MatrixClientSession, PersistableMatrixSession};
use koushi_state::{
    AccountManagementOperation, AppAction, AuthFailureKind, AvatarImage,
    AvatarThumbnailFailureKind, AvatarThumbnailState, CrossSigningStatus, DeviceSessionSummary,
    E2eeRecoveryState, IdentityResetAuthType, IdentityResetState, LoginRequest,
    OperationFailureKind, OwnProfile, PresenceKind, RecoveryKeyDeliveryState, RecoveryMethod,
    RecoveryRequest, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem, SessionInfo,
    TrustOperationFailureKind, VerificationCancelReason, VerificationFlowState, VerificationTarget,
};
use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::events::room::MediaSource as SdkMediaSource;
use matrix_sdk::ruma::{MxcUri, OwnedMxcUri};
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};

use crate::command::{
    AccountCommand, RoomCommand, RoomKeyExportRequest, RoomKeyImportRequest, SearchCommand,
    SecureBackupPassphraseChangeRequest, SecureBackupSetupRequest, SyncCommand, ThreadsListCommand,
    TimelineCommand,
};
use crate::event::{
    AccountEvent, CoreEvent, E2eeTrustEvent, EventCacheFailureReasonClass,
    EventCacheSubscribeStatus, LiveSignalsEvent, LocalEncryptionEvent,
};
use crate::failure::{CoreFailure, LoginFailureKind, ProfileFailureKind, TimelineFailureKind};
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use crate::link_preview::LinkPreviewContext;
use crate::renderable_thumbnail::{
    RenderableThumbnailKind, clear_renderable_thumbnail_cache, store_renderable_thumbnail,
};
use crate::room::{RoomActorHandle, RoomMessage};
use crate::search::SearchActorHandle;
use crate::store::{StoreActor, account_key_from_info, session_key_id_from_info};
use crate::sync::{SyncActorHandle, SyncMessage};
use crate::timeline::{TimelineManagerHandle, TimelineMessage};

/// "Credential store healthy, but no stored session for that account"
/// during restore/switch (canon: `CoreFailure::SessionNotFound`).
const SESSION_NOT_FOUND_FAILURE: CoreFailure = CoreFailure::SessionNotFound;

/// Maximum number of concurrent avatar thumbnail downloads. Bounded to avoid
/// flooding the SDK media layer with parallel requests during large room joins.
const AVATAR_DOWNLOAD_CONCURRENCY: usize = 6;
const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";
const SERVER_LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

/// Redacted message used in reducer error projections (never raw SDK text).
const RESTORE_FAILED_MESSAGE: &str = "session restore failed";
const INCOMING_VERIFICATION_FLOW_ID_BASE: u64 = 1 << 63;

/// Messages routed to the AccountActor task.
pub enum AccountMessage {
    Command(AccountCommand),
    SyncCommand(SyncCommand),
    RoomCommand(RoomCommand),
    TimelineCommand(TimelineCommand),
    ScheduleServerDelayedSend {
        request_id: RequestId,
        scheduled_id: String,
        room_id: String,
        body: String,
        send_at_ms: u64,
    },
    CancelServerDelayedSend {
        request_id: RequestId,
        scheduled_id: String,
        delay_id: String,
    },
    RescheduleServerDelayedSend {
        request_id: RequestId,
        scheduled_id: String,
        room_id: String,
        body: String,
        delay_id: String,
        send_at_ms: u64,
    },
    OpenTimelineAtTimestamp {
        request_id: RequestId,
        room_id: String,
        timestamp_ms: u64,
    },
    SearchCommand(SearchCommand),
    /// Record `AppEffect::NotifySearchCrawlerRoomsAvailable` as a latest-wins
    /// background crawler notification and try to flush it to SearchActor.
    NotifySearchCrawlerRoomsAvailable {
        room_ids: Vec<String>,
        settings: koushi_state::SearchCrawlerSettings,
    },
    /// Forward `AppEffect::InvalidateSearchCrawlerCache` to the actor so it
    /// drops its completed-room cache before the subsequent re-enqueue.
    InvalidateSearchCrawlerCache,
    /// Forward `AppEffect::RebuildSearchIndex` to the actor so it clears local
    /// search documents and crawl queues before re-enqueue.
    RebuildSearchIndex,
    ThreadsListCommand(ThreadsListCommand),
    VerificationRequestProgress {
        request_id: RequestId,
        target: VerificationTarget,
        state: koushi_sdk::MatrixVerificationRequestState,
    },
    SasVerificationProgress {
        request_id: RequestId,
        target: VerificationTarget,
        state: koushi_sdk::MatrixSasState,
    },
    IncomingVerificationRequest {
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    },
    /// Internal: a spawned avatar-fetch task completed. Never exposed to
    /// Tauri/React; carries only the resolved state back into the actor loop.
    /// `generation` matches `AccountActor::avatar_session_generation` at the
    /// time the task was spawned; stale completions (wrong generation after a
    /// session change) are silently dropped by `handle_avatar_fetched`.
    AvatarFetched {
        mxc_uri: String,
        generation: u64,
        thumbnail: AvatarThumbnailState,
    },
    Shutdown,
}

/// Handle to the AccountActor background task.
pub struct AccountActorHandle {
    tx: mpsc::Sender<AccountMessage>,
}

impl AccountActorHandle {
    pub async fn send(&self, msg: AccountMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }
}

/// How a successful store-backed restore is reported.
enum RestoreOutcome {
    /// `RestoreSession` command → `AccountEvent::SessionRestored`.
    Restored,
    /// `SwitchAccount` command → `AccountEvent::AccountSwitched`.
    Switched,
}

struct RecoveryStateObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct VerificationObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct IncomingVerificationObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct PendingVerificationRequest {
    request_id: RequestId,
    target: VerificationTarget,
    handle: koushi_sdk::MatrixVerificationRequestHandle,
}

struct PendingUiaOperation {
    operation: AccountManagementOperation,
    raw_device_ids: Vec<String>,
    new_password: Option<koushi_state::AuthSecret>,
    erase_data: bool,
    uiaa_session: Option<String>,
}

enum AccountManagementUiaError {
    DeleteDevices(koushi_sdk::DeleteDevicesError),
    AccountManagement(koushi_sdk::AccountManagementError),
}

struct PendingSasVerification {
    request_id: RequestId,
    target: VerificationTarget,
    handle: koushi_sdk::MatrixSasVerificationHandle,
}

/// The account actor's internal state.
pub struct AccountActor {
    /// Active store-backed session, if any.
    session: Option<Arc<MatrixClientSession>>,
    /// Session key for credential store operations.
    session_key_id: Option<SessionKeyId>,
    /// Store actor — owns the credential store backend and per-account paths.
    store: StoreActor,
    /// App-level action channel to drive the reducer.
    action_tx: mpsc::Sender<Vec<AppAction>>,
    /// Shared event broadcast channel.
    event_tx: broadcast::Sender<CoreEvent>,
    /// Message inbox.
    command_rx: mpsc::Receiver<AccountMessage>,
    /// Sender clone used by SDK observation tasks to report actor-owned
    /// verification progress back into this actor's mailbox.
    self_tx: mpsc::Sender<AccountMessage>,
    /// SyncActor child handle (Phase 3). Present only when a store-backed
    /// session exists. Created on first login/restore; destroyed on logout /
    /// account switch.
    sync_actor: Option<SyncActorHandle>,
    /// RoomActor child handle (Phase 4). Spawned once at actor creation and
    /// kept alive for the lifetime of the AccountActor. Session is provided
    /// via `RoomMessage::SyncStarted` when sync begins.
    room_actor: RoomActorHandle,
    /// TimelineManagerActor handle (Phase 5). Spawned once at actor creation;
    /// session reference is updated when a store-backed session is established.
    timeline_manager: TimelineManagerHandle,
    /// Account-wide gate for `/rooms/{roomId}/messages` requests. Timeline
    /// pagination has priority over background search-history crawling.
    messages_backpressure: crate::messages_backpressure::MessagesBackpressure,
    /// Application data directory for cached preview images.
    data_dir: std::path::PathBuf,
    /// Latest link-preview policy snapshot from AppState, kept current so a
    /// newly-created session-scoped timeline manager starts with the right policy.
    link_preview_policy: LinkPreviewContext,
    /// SearchActor handle (Phase 6). Present only when a store-backed session
    /// exists. Created at the same time as SyncActor; stopped in the ordered
    /// shutdown between timelines and sync (canon Async rule 12 step 3).
    search_actor: Option<SearchActorHandle>,
    /// ThreadsListActor handle. Present only while the threads list view is
    /// open. Dropping the handle cancels the actor and its SDK subscriptions.
    threads_list_actor: Option<crate::threads_list::ThreadsListActorHandle>,
    /// Recovery-state observer task for the active store-backed session.
    recovery_observer: Option<RecoveryStateObservation>,
    /// Pending SDK identity reset continuation, held only inside AccountActor.
    identity_reset_handle: Option<koushi_sdk::MatrixIdentityResetHandle>,
    /// Actor-private mapping from app-owned device ordinal to raw Matrix
    /// device id. Raw ids never enter reducer state or snapshots.
    device_session_ordinals: BTreeMap<u64, String>,
    /// Pending UIA operations keyed by the flow id (original request id).
    /// Holds the data needed to retry a destructive action after the user
    /// supplies interactive auth. Secrets (password, UIA session) are held
    /// only inside this actor-private map, never in reducer state.
    pending_uia_operations: BTreeMap<u64, PendingUiaOperation>,
    /// Pending SDK verification request continuation, held only inside
    /// AccountActor and never projected into AppState.
    verification_request: Option<PendingVerificationRequest>,
    /// Pending SDK SAS continuation, held only inside AccountActor and never
    /// projected into AppState.
    sas_verification: Option<PendingSasVerification>,
    /// SDK verification request observer task for the active flow.
    verification_request_observer: Option<VerificationObservation>,
    /// SDK SAS observer task for the active flow.
    sas_verification_observer: Option<VerificationObservation>,
    /// SDK incoming verification request observer for the active session.
    incoming_verification_observer: Option<IncomingVerificationObservation>,
    /// Synthetic flow id sequence for SDK-originated verification requests.
    next_incoming_verification_sequence: u64,
    /// Last `NotifySearchCrawlerRoomsAvailable` payload received before the
    /// `SearchActor` was spawned.  Replayed into the actor immediately after
    /// it is created so rooms that were already known to the reducer at
    /// session-restore time are not missed by the auto-start logic.
    pending_crawler_notification: Option<(Vec<String>, koushi_state::SearchCrawlerSettings)>,
    /// Actor-owned avatar thumbnail cache: mxc_uri -> last resolved state.
    /// Mutated only from the actor loop; no shared lock needed.
    avatar_cache: HashMap<String, AvatarThumbnailState>,
    /// In-flight fetches: mxc_uri -> waiting request_ids (single-flight dedup).
    /// The first `DownloadAvatarThumbnail` for a given mxc spawns a task and
    /// records its `request_id` here; subsequent ones for the same mxc while
    /// the task is running simply append their `request_id`. When `AvatarFetched`
    /// arrives every waiter receives `AvatarThumbnailDownloaded`.
    /// Entries are removed (and all waiters notified) when `AvatarFetched` arrives.
    avatar_inflight: HashMap<String, Vec<RequestId>>,
    /// Semaphore bounding concurrent avatar downloads. Cloned into spawned
    /// fetch tasks; the actor holds one Arc so it can be replaced on session
    /// clear.
    avatar_download_semaphore: Arc<Semaphore>,
    /// Owns all spawned avatar-fetch tasks. Aborted on session clear and
    /// shutdown (engineering-rules: every spawned task has an owner).
    avatar_fetch_tasks: tokio::task::JoinSet<()>,
    /// Incremented by `abort_avatar_fetch_tasks` on every session clear /
    /// logout / switch / shutdown so that `AvatarFetched` completions that
    /// were already enqueued before the abort are detected and silently dropped
    /// instead of being accepted into the new (or absent) session's state.
    avatar_session_generation: u64,
}

impl AccountActor {
    pub fn spawn(
        store_actor: StoreActor,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        initial_link_preview_policy: LinkPreviewContext,
    ) -> AccountActorHandle {
        // AppActor forwards every Room/Timeline/Sync command here via send().await;
        // sized so heavy sync does not block the AppActor's forwarding.
        let (tx, command_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let data_dir = store_actor.data_dir().to_path_buf();
        // Spawn RoomActor once at AccountActor creation. It starts with no
        // session and waits for RoomMessage::SyncStarted.
        let room_actor = crate::room::RoomActor::spawn(action_tx.clone(), event_tx.clone());
        let messages_backpressure = crate::messages_backpressure::MessagesBackpressure::default();
        // Spawn TimelineManagerActor. It starts with no session; the session
        // is injected when a store-backed session is established.
        let timeline_manager = crate::timeline::TimelineManagerActor::spawn(
            action_tx.clone(),
            event_tx.clone(),
            Some(data_dir.clone()),
            messages_backpressure.clone(),
        );
        let actor = AccountActor {
            session: None,
            session_key_id: None,
            store: store_actor,
            action_tx,
            event_tx,
            command_rx,
            self_tx: tx.clone(),
            sync_actor: None,
            room_actor,
            timeline_manager,
            messages_backpressure,
            data_dir,
            link_preview_policy: initial_link_preview_policy,
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            device_session_ordinals: BTreeMap::new(),
            pending_uia_operations: BTreeMap::new(),
            verification_request: None,
            sas_verification: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            incoming_verification_observer: None,
            next_incoming_verification_sequence: INCOMING_VERIFICATION_FLOW_ID_BASE,
            pending_crawler_notification: None,
            avatar_cache: HashMap::new(),
            avatar_inflight: HashMap::new(),
            avatar_download_semaphore: Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY)),
            avatar_fetch_tasks: tokio::task::JoinSet::new(),
            avatar_session_generation: 0,
        };
        crate::executor::spawn(actor.run());
        AccountActorHandle { tx }
    }

    async fn run(mut self) {
        while let Some(msg) = self.command_rx.recv().await {
            match msg {
                AccountMessage::Shutdown => break,
                AccountMessage::Command(command) => {
                    self.handle_command(command).await;
                }
                AccountMessage::SyncCommand(sync_command) => {
                    self.route_sync_command(sync_command).await;
                }
                AccountMessage::RoomCommand(room_command) => {
                    self.route_room_command(room_command).await;
                }
                AccountMessage::TimelineCommand(timeline_command) => {
                    self.route_timeline_command(timeline_command).await;
                }
                AccountMessage::ScheduleServerDelayedSend {
                    request_id,
                    scheduled_id,
                    room_id,
                    body,
                    send_at_ms,
                } => {
                    self.handle_schedule_server_delayed_send(
                        request_id,
                        scheduled_id,
                        room_id,
                        body,
                        send_at_ms,
                    )
                    .await;
                }
                AccountMessage::CancelServerDelayedSend {
                    request_id,
                    scheduled_id,
                    delay_id,
                } => {
                    self.handle_cancel_server_delayed_send(request_id, scheduled_id, delay_id)
                        .await;
                }
                AccountMessage::RescheduleServerDelayedSend {
                    request_id,
                    scheduled_id,
                    room_id,
                    body,
                    delay_id,
                    send_at_ms,
                } => {
                    self.handle_reschedule_server_delayed_send(
                        request_id,
                        scheduled_id,
                        room_id,
                        body,
                        delay_id,
                        send_at_ms,
                    )
                    .await;
                }
                AccountMessage::OpenTimelineAtTimestamp {
                    request_id,
                    room_id,
                    timestamp_ms,
                } => {
                    self.handle_open_timeline_at_timestamp(request_id, room_id, timestamp_ms)
                        .await;
                }
                AccountMessage::SearchCommand(search_command) => {
                    self.route_search_command(search_command).await;
                }
                AccountMessage::NotifySearchCrawlerRoomsAvailable { room_ids, settings } => {
                    // Room availability for the crawler is latest-wins
                    // background state. Store it first, then try a
                    // non-blocking flush so AccountActor does not stall room
                    // or timeline commands behind crawler mailbox pressure.
                    self.pending_crawler_notification = Some((room_ids, settings));
                    self.flush_pending_crawler_notification();
                }
                AccountMessage::InvalidateSearchCrawlerCache => {
                    if let Some(handle) = &self.search_actor {
                        handle.invalidate_crawler_cache().await;
                    }
                    // If the actor is not yet running there is no completed-room
                    // cache to clear; the pending_crawler_notification is
                    // already the latest settings so a new crawl will use them.
                }
                AccountMessage::RebuildSearchIndex => {
                    if let Some(handle) = &self.search_actor {
                        handle.rebuild_search_index().await;
                    }
                }
                AccountMessage::ThreadsListCommand(threads_list_command) => {
                    self.route_threads_list_command(threads_list_command).await;
                }
                AccountMessage::VerificationRequestProgress {
                    request_id,
                    target,
                    state,
                } => {
                    self.handle_verification_request_progress(request_id, target, state)
                        .await;
                }
                AccountMessage::SasVerificationProgress {
                    request_id,
                    target,
                    state,
                } => {
                    self.handle_sas_verification_progress(request_id, target, state)
                        .await;
                }
                AccountMessage::IncomingVerificationRequest { target, handle } => {
                    let request_id = self.next_incoming_verification_request_id();
                    self.handle_incoming_verification_request(request_id, target, handle)
                        .await;
                }
                AccountMessage::AvatarFetched {
                    mxc_uri,
                    generation,
                    thumbnail,
                } => {
                    self.handle_avatar_fetched(mxc_uri, generation, thumbnail)
                        .await;
                }
            }
            self.flush_pending_crawler_notification();
        }
        // Ordered shutdown (overview.md Async rule 12):
        // recovery/incoming observers → timelines → search → room → sync → SDK handles.
        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        self.stop_timeline_actor().await;
        self.stop_threads_list_actor().await;
        self.stop_search_actor().await;
        self.stop_room_actor().await;
        self.stop_sync_actor().await;
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;
        self.abort_avatar_fetch_tasks();
        self.device_session_ordinals.clear();
        self.pending_uia_operations.clear();
        // Drop the session handle inside the runtime context
        // (overview.md Async rule 11 — deadpool-runtime panic prevention).
        drop(self.session.take());
    }

    /// Route a RoomCommand to the RoomActor. The RoomActor handles the
    /// SessionRequired check internally (it holds the session ref after
    /// SyncStarted).
    async fn route_room_command(&self, command: RoomCommand) {
        trace_room_route("send", &command);
        let sent = self.room_actor.send(RoomMessage::Command(command)).await;
        if !sent {
            trace_room_route_closed();
        }
    }

    /// Route a TimelineCommand to the TimelineManagerActor.
    /// Session guard is enforced by AppActor before routing; AccountActor
    /// passes through directly to avoid double-gating.
    async fn route_timeline_command(&mut self, command: TimelineCommand) {
        if let TimelineCommand::BroadcastLinkPreviewPolicy {
            unencrypted_global_enabled,
            encrypted_global_enabled,
            room_overrides,
        } = &command
        {
            self.link_preview_policy.unencrypted_global_enabled = *unencrypted_global_enabled;
            self.link_preview_policy.encrypted_global_enabled = *encrypted_global_enabled;
            self.link_preview_policy.room_overrides = room_overrides.clone();
        }
        let _ = self
            .timeline_manager
            .send(TimelineMessage::Command(command))
            .await;
    }

    fn flush_pending_crawler_notification(&mut self) {
        let Some(handle) = &self.search_actor else {
            return;
        };
        let Some((room_ids, settings)) = self.pending_crawler_notification.take() else {
            return;
        };
        if let Err((room_ids, settings)) = handle.try_notify_rooms_available(room_ids, settings) {
            self.pending_crawler_notification = Some((room_ids, settings));
        }
    }

    /// Route a SearchCommand to the SearchActor. Emit SessionRequired if no
    /// search actor is active.
    async fn route_search_command(&self, command: SearchCommand) {
        let request_id = match &command {
            SearchCommand::Query { request_id, .. }
            | SearchCommand::Attachments { request_id, .. }
            | SearchCommand::StartHistoryCrawl { request_id, .. }
            | SearchCommand::StopHistoryCrawl { request_id, .. } => *request_id,
        };
        match &self.search_actor {
            Some(handle) => {
                if !handle.send_command(command).await {
                    self.emit_search_failed(request_id, SEARCH_UNAVAILABLE_MESSAGE)
                        .await;
                    self.emit_failure(request_id, CoreFailure::SessionRequired);
                }
            }
            None => {
                self.emit_search_failed(request_id, SEARCH_UNAVAILABLE_MESSAGE)
                    .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
        }
    }

    /// Route a ThreadsListCommand to the ThreadsListActor. Spawns the actor
    /// on `Open` when a session is present; drops it on `Close`.
    async fn route_threads_list_command(&mut self, command: ThreadsListCommand) {
        match command {
            ThreadsListCommand::Open {
                request_id,
                room_id,
            } => {
                let Some(session) = self.session.clone() else {
                    self.emit_threads_list_failed(request_id, room_id).await;
                    self.emit_failure(request_id, CoreFailure::SessionRequired);
                    return;
                };
                if self
                    .threads_list_actor
                    .as_ref()
                    .map(|handle| handle.room_id() != room_id)
                    .unwrap_or(false)
                {
                    self.threads_list_actor = None;
                }
                if self.threads_list_actor.is_none() {
                    self.threads_list_actor = Some(crate::threads_list::ThreadsListActor::spawn(
                        session,
                        self.action_tx.clone(),
                        self.event_tx.clone(),
                        room_id.clone(),
                    ));
                }
                if let Some(handle) = &self.threads_list_actor {
                    let _ = handle.open(request_id, room_id).await;
                }
            }
            ThreadsListCommand::Close { request_id } => {
                if let Some(handle) = self.threads_list_actor.take() {
                    let _ = handle.close(request_id).await;
                }
            }
            ThreadsListCommand::Paginate {
                request_id,
                room_id,
            } => {
                if let Some(handle) = &self.threads_list_actor {
                    if handle.room_id() == room_id {
                        let _ = handle.paginate(request_id).await;
                    }
                }
            }
        }
    }

    async fn handle_schedule_server_delayed_send(
        &self,
        request_id: RequestId,
        scheduled_id: String,
        room_id: String,
        body: String,
        send_at_ms: u64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let capability = crate::scheduled_send::detect_capability(&session.client()).await;
        if capability == ScheduledSendCapability::ServerDelayedEvents {
            match self
                .send_server_delayed_message(session, &room_id, &body, send_at_ms)
                .await
            {
                Ok(delay_id) => {
                    self.reduce(vec![
                        AppAction::ScheduledSendCapabilityChanged {
                            capability: ScheduledSendCapability::ServerDelayedEvents,
                        },
                        AppAction::ScheduledSendCreated {
                            item: ScheduledSendItem {
                                scheduled_id,
                                room_id,
                                body,
                                send_at_ms,
                                handle: ScheduledSendHandle::Server { delay_id },
                            },
                        },
                    ]);
                    return;
                }
                Err(()) => {}
            }
        }

        self.reduce(vec![
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
            AppAction::ScheduledSendCreated {
                item: ScheduledSendItem {
                    scheduled_id,
                    room_id,
                    body,
                    send_at_ms,
                    handle: ScheduledSendHandle::Local,
                },
            },
        ]);
    }

    async fn handle_cancel_server_delayed_send(
        &self,
        request_id: RequestId,
        scheduled_id: String,
        delay_id: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match self
            .update_server_delayed_event(
                session,
                delay_id,
                matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::unstable::UpdateAction::Cancel,
            )
            .await
        {
            Ok(()) => self.reduce(vec![AppAction::ScheduledSendCancelled { scheduled_id }]),
            Err(()) => self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            ),
        }
    }

    async fn handle_reschedule_server_delayed_send(
        &self,
        request_id: RequestId,
        scheduled_id: String,
        room_id: String,
        body: String,
        delay_id: String,
        send_at_ms: u64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        if self
            .update_server_delayed_event(
                session,
                delay_id,
                matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::unstable::UpdateAction::Cancel,
            )
            .await
            .is_err()
        {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }

        match self
            .send_server_delayed_message(session, &room_id, &body, send_at_ms)
            .await
        {
            Ok(delay_id) => {
                self.reduce(vec![AppAction::ScheduledSendRescheduled {
                    scheduled_id,
                    send_at_ms,
                    handle: ScheduledSendHandle::Server { delay_id },
                }]);
            }
            Err(()) => {
                self.reduce(vec![
                    AppAction::ScheduledSendCapabilityChanged {
                        capability: ScheduledSendCapability::LocalFallback,
                    },
                    AppAction::ScheduledSendRescheduled {
                        scheduled_id,
                        send_at_ms,
                        handle: ScheduledSendHandle::Local,
                    },
                ]);
            }
        }
    }

    async fn send_server_delayed_message(
        &self,
        session: &MatrixClientSession,
        room_id: &str,
        body: &str,
        send_at_ms: u64,
    ) -> Result<String, ()> {
        use matrix_sdk::ruma::TransactionId;
        use matrix_sdk::ruma::api::client::delayed_events::{
            DelayParameters, delayed_message_event,
        };
        use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

        let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| ())?;
        let content = RoomMessageEventContent::text_plain(body.to_owned());
        let request = delayed_message_event::unstable::Request::new(
            room_id,
            TransactionId::new(),
            DelayParameters::Timeout {
                timeout: crate::scheduled_send::server_delay_timeout(
                    send_at_ms,
                    crate::scheduled_send::current_epoch_ms(),
                ),
            },
            &content,
        )
        .map_err(|_| ())?;

        session
            .client()
            .send(request)
            .await
            .map(|response| response.delay_id)
            .map_err(|_| ())
    }

    async fn update_server_delayed_event(
        &self,
        session: &MatrixClientSession,
        delay_id: String,
        action: matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::unstable::UpdateAction,
    ) -> Result<(), ()> {
        let request =
            matrix_sdk::ruma::api::client::delayed_events::update_delayed_event::unstable::Request::new(
                delay_id, action,
            );
        session
            .client()
            .send(request)
            .await
            .map(|_| ())
            .map_err(|_| ())
    }

    async fn emit_search_failed(&self, request_id: RequestId, message: &str) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
                message: message.to_owned(),
            }])
            .await;
    }

    async fn emit_threads_list_failed(&self, request_id: RequestId, room_id: String) {
        let _ = self
            .action_tx
            .send(vec![AppAction::ThreadsListFailed {
                request_id: request_id.sequence,
                room_id,
                failure_kind: OperationFailureKind::Network,
            }])
            .await;
    }

    async fn handle_open_timeline_at_timestamp(
        &mut self,
        request_id: RequestId,
        room_id: String,
        timestamp_ms: u64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let Some(account_key) = self.active_account_key() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let parsed_room_id = match matrix_sdk::ruma::RoomId::parse(room_id.as_str()) {
            Ok(room_id) => room_id,
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

        let request = Self::timeline_event_by_timestamp_request(parsed_room_id, timestamp_ms);
        let response = match session.client().send(request).await {
            Ok(response) => response,
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
        let event_id = response.event_id.to_string();
        let _ = self
            .action_tx
            .send(vec![AppAction::OpenFocusedContext {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            }])
            .await;
        self.route_timeline_command(TimelineCommand::Subscribe {
            request_id,
            key: TimelineKey {
                account_key,
                kind: TimelineKind::Focused { room_id, event_id },
            },
        })
        .await;
    }

    /// Ordered shutdown of the SearchActor (step 3 of the shutdown sequence,
    /// after timelines and before sync — canon Async rule 12 step 3).
    async fn stop_search_actor(&mut self) {
        // Clear any buffered notification so it is not replayed for the next
        // session after logout or account switch.
        self.pending_crawler_notification = None;
        if let Some(handle) = self.search_actor.take() {
            handle.shutdown().await;
        }
    }

    async fn stop_current_session_runtime(&mut self) {
        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        self.stop_timeline_actor().await;
        self.stop_threads_list_actor().await;
        self.stop_search_actor().await;
        self.stop_sync_actor().await;
        self.clear_room_actor_session().await;
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;
        self.abort_avatar_fetch_tasks();
        self.device_session_ordinals.clear();
        self.pending_uia_operations.clear();
    }

    /// Ordered shutdown of the ThreadsListActor. Dropping the handle cancels
    /// the actor and its SDK subscriptions.
    async fn stop_threads_list_actor(&mut self) {
        let _ = self.threads_list_actor.take();
    }

    /// Ordered shutdown of the TimelineManagerActor (step 2 of the shutdown
    /// sequence per Async rule 12 — timelines before search/room/sync).
    async fn stop_timeline_actor(&mut self) {
        let _ = self.timeline_manager.send(TimelineMessage::Shutdown).await;
    }

    /// Route a SyncCommand to the SyncActor, or emit SessionRequired if no
    /// store-backed session is active yet.
    async fn route_sync_command(&mut self, command: SyncCommand) {
        let request_id = match &command {
            SyncCommand::Start { request_id }
            | SyncCommand::Stop { request_id }
            | SyncCommand::Restart { request_id }
            | SyncCommand::SyncOnce { request_id } => *request_id,
        };

        if self.sync_actor.is_none()
            && let Some(session) = &self.session
        {
            self.spawn_sync_actor(session.clone()).await;
        }

        match &self.sync_actor {
            Some(handle) => {
                // The SyncActor notifies the RoomActor itself on start/stop/
                // restart: only it knows the selected backend and owns the
                // live RoomListService (canon, overview.md RoomActor bullet).
                let _ = handle.send(SyncMessage::Command(command)).await;
            }
            None => {
                // Session not yet ready — gate is enforced in AppActor but be
                // defensive here too.
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
        }
    }

    /// Spawn the SyncActor for the just-established store-backed session and
    /// notify the RoomActor so room operations become available.
    /// Also replace the TimelineManagerActor with one that holds the session.
    /// Also spawn the SearchActor (Phase 6).
    async fn spawn_sync_actor(&mut self, session: Arc<MatrixClientSession>) {
        // Give the RoomActor the session so room ops work even before sync
        // starts. The room-list observation starts later, on the SyncActor's
        // RoomMessage::SyncStarted (which carries the live RoomListService on
        // the SyncService backend). try_send: this is a sync fn; capacity 64
        // is more than enough for this one message.
        self.room_actor.try_send(RoomMessage::SessionEstablished {
            session: session.clone(),
        });

        // Spawn SearchActor (Phase 6). The session already holds the search
        // index (configured in restore_into_store / the client builder). The
        // search actor gets an mpsc::Sender<SearchIndexMessage> which will be
        // forwarded to the TimelineManagerActor below.
        let search_handle = crate::search::SearchActor::spawn(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.messages_backpressure.clone(),
        );
        let search_index_tx = search_handle.index_sender();

        self.search_actor = Some(search_handle);
        // Replay any notification that arrived before the actor was ready so
        // rooms already known to the reducer at session-restore time are not
        // missed by the auto-start logic. Flush is non-blocking; if the search
        // actor is already saturated, the latest payload remains pending for
        // the next AccountActor tick.
        self.flush_pending_crawler_notification();

        // Replace the TimelineManagerActor with one holding the current session
        // AND the search index sender. The old manager (with no session) is
        // stopped by dropping its handle. We use try_send to shut down the old.
        self.timeline_manager.try_send(TimelineMessage::Shutdown);
        self.timeline_manager = crate::timeline::TimelineManagerActor::spawn_with_session(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            search_index_tx,
            Some(self.data_dir.clone()),
            self.link_preview_policy.clone(),
            self.messages_backpressure.clone(),
        );

        let handle = crate::sync::SyncActor::spawn(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.room_actor.tx.clone(),
            self.timeline_manager.sender(),
        );
        self.sync_actor = Some(handle);
        self.start_scheduled_send_capability_probe(session);
    }

    fn start_scheduled_send_capability_probe(&self, session: Arc<MatrixClientSession>) {
        let action_tx = self.action_tx.clone();
        crate::executor::spawn(async move {
            let capability = crate::scheduled_send::detect_capability(&session.client()).await;
            let _ = action_tx
                .send(vec![AppAction::ScheduledSendCapabilityChanged {
                    capability,
                }])
                .await;
        });
    }

    fn start_recovery_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = crate::executor::spawn(run_recovery_state_observation(
            session.e2ee_recovery_state_stream(),
            account_key_from_info(&session.info),
            self.action_tx.clone(),
            self.event_tx.clone(),
            stop_rx,
        ));
        self.recovery_observer = Some(RecoveryStateObservation { stop_tx, task });
    }

    fn start_incoming_verification_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut observer = koushi_sdk::observe_incoming_verification_requests(&session);
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    request = observer.recv() => {
                        let Some(request) = request else { break };
                        let (target, handle) = request.into_parts();
                        if tx
                            .send(AccountMessage::IncomingVerificationRequest {
                                target,
                                handle,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        self.incoming_verification_observer =
            Some(IncomingVerificationObservation { stop_tx, task });
    }

    fn next_incoming_verification_request_id(&mut self) -> RequestId {
        let sequence = self.next_incoming_verification_sequence;
        self.next_incoming_verification_sequence = self
            .next_incoming_verification_sequence
            .checked_add(1)
            .unwrap_or(INCOMING_VERIFICATION_FLOW_ID_BASE);
        incoming_verification_request_id(sequence)
    }

    /// Ordered shutdown of the SyncActor (step 4 of the shutdown sequence).
    async fn stop_sync_actor(&mut self) {
        if let Some(handle) = self.sync_actor.take() {
            let _ = handle.shutdown().await;
        }
    }

    async fn stop_recovery_observer(&mut self) {
        if let Some(observation) = self.recovery_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn stop_incoming_verification_observer(&mut self) {
        if let Some(observation) = self.incoming_verification_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn cancel_identity_reset_handle(&mut self) {
        if let Some(handle) = self.identity_reset_handle.take() {
            handle.cancel().await;
        }
    }

    async fn stop_verification_request_observer(&mut self) {
        if let Some(observation) = self.verification_request_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn stop_sas_verification_observer(&mut self) {
        if let Some(observation) = self.sas_verification_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn cancel_verification_handles(&mut self) {
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        if let Some(pending) = self.sas_verification.take() {
            let _ = koushi_sdk::cancel_sas_verification(&pending.handle).await;
        }
        if let Some(pending) = self.verification_request.take() {
            let _ = koushi_sdk::cancel_verification_request(&pending.handle).await;
        }
    }

    /// Ordered shutdown of the RoomActor (before sync stop in the shutdown
    /// sequence). The RoomActor is not Option<> since it is always present;
    /// we send Shutdown and the task finishes on its own after processing it.
    async fn stop_room_actor(&mut self) {
        let _ = self.room_actor.send(RoomMessage::Shutdown).await;
    }

    async fn clear_room_actor_session(&mut self) {
        let _ = self.room_actor.send(RoomMessage::SessionCleared).await;
    }

    async fn handle_command(&mut self, command: AccountCommand) {
        match command {
            AccountCommand::DiscoverLogin {
                request_id,
                homeserver,
            } => {
                self.handle_discover_login(request_id, homeserver).await;
            }
            AccountCommand::StartOidcLogin {
                request_id,
                homeserver,
            } => {
                self.handle_start_oidc_login(request_id, homeserver).await;
            }
            AccountCommand::CompleteOidcLogin {
                request_id,
                homeserver,
                callback_url,
            } => {
                self.handle_complete_oidc_login(request_id, homeserver, callback_url)
                    .await;
            }
            AccountCommand::LoginPassword {
                request_id,
                request,
            } => {
                self.handle_login_password(request_id, request).await;
            }
            AccountCommand::RestoreSession {
                request_id,
                account_key,
            } => {
                self.handle_restore_session(request_id, account_key).await;
            }
            AccountCommand::RestoreLastSession { request_id } => {
                self.handle_restore_last_session(request_id).await;
            }
            AccountCommand::QuerySavedSessions { request_id } => {
                self.handle_query_saved_sessions(request_id);
            }
            AccountCommand::QueryDevices { request_id } => {
                self.handle_query_devices(request_id).await;
            }
            AccountCommand::LoadAccountManagementCapabilities { request_id } => {
                self.handle_load_account_management_capabilities(request_id)
                    .await;
            }
            AccountCommand::RenameDevice {
                request_id,
                device_ordinal,
                display_name,
            } => {
                self.handle_rename_device(request_id, device_ordinal, display_name)
                    .await;
            }
            AccountCommand::DeleteDevices {
                request_id,
                device_ordinals,
                auth,
            } => {
                self.handle_delete_devices(request_id, device_ordinals, auth)
                    .await;
            }
            AccountCommand::ChangePassword {
                request_id,
                new_password,
            } => {
                self.handle_change_password(request_id, new_password).await;
            }
            AccountCommand::DeactivateAccount {
                request_id,
                erase_data,
            } => {
                self.handle_deactivate_account(request_id, erase_data).await;
            }
            AccountCommand::SubmitAccountManagementUia {
                request_id,
                flow_id,
                auth,
            } => {
                self.handle_submit_account_management_uia(request_id, flow_id, auth)
                    .await;
            }
            AccountCommand::SoftLogoutReauth {
                request_id,
                password,
            } => {
                self.handle_soft_logout_reauth(request_id, password).await;
            }
            AccountCommand::ExportRoomKeys {
                request_id,
                request,
            } => {
                self.handle_export_room_keys(request_id, request).await;
            }
            AccountCommand::ImportRoomKeys {
                request_id,
                request,
            } => {
                self.handle_import_room_keys(request_id, request).await;
            }
            AccountCommand::BootstrapSecureBackup {
                request_id,
                request,
            } => {
                self.handle_bootstrap_secure_backup(request_id, request)
                    .await;
            }
            AccountCommand::ChangeSecureBackupPassphrase {
                request_id,
                request,
            } => {
                self.handle_change_secure_backup_passphrase(request_id, request)
                    .await;
            }
            AccountCommand::ProbeLocalEncryptionHealth { request_id } => {
                self.handle_probe_local_encryption_health(request_id);
            }
            AccountCommand::ResetLocalData { request_id } => {
                self.handle_reset_local_data(request_id).await;
            }
            AccountCommand::Logout { request_id } => {
                self.handle_logout(request_id).await;
            }
            AccountCommand::SwitchAccount {
                request_id,
                account_key,
            } => {
                self.handle_switch_account(request_id, account_key).await;
            }
            AccountCommand::SubmitRecovery {
                request_id,
                request,
            } => {
                self.handle_submit_recovery(request_id, request).await;
            }
            AccountCommand::BootstrapCrossSigning { request_id, auth } => {
                self.handle_bootstrap_cross_signing(request_id, auth).await;
            }
            AccountCommand::EnableKeyBackup {
                request_id,
                passphrase,
            } => {
                self.handle_enable_key_backup(request_id, passphrase).await;
            }
            AccountCommand::RestoreKeyBackup {
                request_id,
                version,
                request,
            } => {
                self.handle_restore_key_backup(request_id, version, request)
                    .await;
            }
            AccountCommand::ResetIdentity { request_id } => {
                self.handle_reset_identity(request_id).await;
            }
            AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request,
            } => {
                self.handle_submit_identity_reset_auth(request_id, flow_id, request)
                    .await;
            }
            AccountCommand::SetPresence {
                request_id,
                presence,
            } => {
                self.handle_set_presence(request_id, presence).await;
            }
            AccountCommand::SetDisplayName {
                request_id,
                display_name,
            } => {
                self.handle_set_display_name(request_id, display_name).await;
            }
            AccountCommand::SetLocalUserAlias {
                request_id,
                user_id,
                alias,
            } => {
                self.handle_set_local_user_alias(request_id, user_id, alias)
                    .await;
            }
            AccountCommand::SetAvatar {
                request_id,
                request,
            } => {
                self.handle_set_avatar(request_id, request).await;
            }
            AccountCommand::DownloadAvatarThumbnail {
                request_id,
                mxc_uri,
            } => {
                self.handle_download_avatar_thumbnail(request_id, mxc_uri)
                    .await;
            }
            AccountCommand::IgnoreUser {
                request_id,
                user_id,
            } => {
                self.handle_ignore_user(request_id, user_id, true).await;
            }
            AccountCommand::UnignoreUser {
                request_id,
                user_id,
            } => {
                self.handle_ignore_user(request_id, user_id, false).await;
            }
            AccountCommand::ReportUser {
                request_id,
                user_id,
                reason,
            } => {
                self.handle_report_user(request_id, user_id, reason).await;
            }
            AccountCommand::RequestVerification { request_id, target } => {
                self.handle_request_verification(request_id, target).await;
            }
            AccountCommand::AcceptVerification {
                request_id,
                flow_id,
            } => {
                self.handle_accept_verification(request_id, flow_id).await;
            }
            AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            } => {
                self.handle_confirm_sas_verification(request_id, flow_id)
                    .await;
            }
            AccountCommand::CancelVerification {
                request_id,
                flow_id,
                reason,
            } => {
                self.handle_cancel_verification(request_id, flow_id, reason)
                    .await;
            }
        }
    }

    async fn handle_request_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::VerificationFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        self.cancel_verification_handles().await;
        match koushi_sdk::request_device_verification(&session, &target).await {
            Ok(handle) => {
                self.verification_request = Some(PendingVerificationRequest {
                    request_id,
                    target: target.clone(),
                    handle: handle.clone(),
                });
                self.observe_verification_request(request_id, target.clone(), handle.clone());
                self.reduce(vec![AppAction::VerificationRequested {
                    request_id: request_id.sequence,
                    target: target.clone(),
                }]);
                self.emit_verification_progress(VerificationFlowState::Requested {
                    request_id: request_id.sequence,
                    target,
                });
                self.project_verification_request_state(request_id, handle.state())
                    .await;
            }
            Err(error) => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    classify_e2ee_trust_error(&error),
                )
                .await;
            }
        }
    }

    async fn handle_incoming_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    ) {
        if self
            .verification_request
            .as_ref()
            .is_some_and(|pending| pending.handle.flow_id() == handle.flow_id())
        {
            return;
        }

        if self.verification_request.is_some() || self.sas_verification.is_some() {
            let _ = koushi_sdk::cancel_verification_request(&handle).await;
            return;
        }

        self.verification_request = Some(PendingVerificationRequest {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.observe_verification_request(request_id, target.clone(), handle.clone());
        self.reduce(vec![AppAction::VerificationRequested {
            request_id: request_id.sequence,
            target: target.clone(),
        }]);
        self.emit_verification_progress(VerificationFlowState::Requested {
            request_id: request_id.sequence,
            target,
        });
        self.project_verification_request_state(request_id, handle.state())
            .await;
    }

    async fn handle_set_presence(&self, request_id: RequestId, presence: PresenceKind) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let user_id = session.info.user_id.clone();
        let _ = self
            .action_tx
            .send(vec![AppAction::PresenceUpdated {
                user_id: user_id.clone(),
                presence,
            }])
            .await;
        self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::PresenceSet {
            request_id,
            presence,
        }));
        self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::PresenceUpdated {
            user_id,
            presence,
        }));
    }

    async fn handle_set_display_name(&self, request_id: RequestId, display_name: Option<String>) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::ProfileUpdateFailed {
                request_id: request_id.sequence,
                message: "profile update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::set_display_name(session, display_name.as_deref()).await {
            Ok(profile) => {
                let profile = map_matrix_own_profile(profile);
                self.send_actions(vec![AppAction::ProfileUpdateSucceeded {
                    request_id: request_id.sequence,
                    profile,
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::ProfileUpdated {
                    request_id,
                    account_key: AccountKey(session.info.user_id.clone()),
                }));
            }
            Err(error) => {
                self.send_actions(vec![AppAction::ProfileUpdateFailed {
                    request_id: request_id.sequence,
                    message: "profile update failed".to_owned(),
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::ProfileOperationFailed {
                        kind: classify_profile_error(&error),
                    },
                );
            }
        }
    }

    async fn handle_set_local_user_alias(
        &self,
        request_id: RequestId,
        user_id: String,
        alias: Option<String>,
    ) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::LocalUserAliasUpdateFailed {
                request_id: request_id.sequence,
                message: "local alias update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::update_local_user_alias(session, &user_id, alias.as_deref()).await {
            Ok(aliases) => {
                self.send_actions(vec![
                    AppAction::LocalUserAliasUpdateSucceeded {
                        request_id: request_id.sequence,
                    },
                    AppAction::LocalUserAliasesLoaded {
                        aliases: aliases.aliases,
                    },
                ])
                .await;
            }
            Err(error) => {
                self.send_actions(vec![AppAction::LocalUserAliasUpdateFailed {
                    request_id: request_id.sequence,
                    message: "local alias update failed".to_owned(),
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::ProfileOperationFailed {
                        kind: classify_profile_error(&error),
                    },
                );
            }
        }
    }

    async fn handle_set_avatar(
        &self,
        request_id: RequestId,
        request: crate::command::SetAvatarRequest,
    ) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::ProfileUpdateFailed {
                request_id: request_id.sequence,
                message: "profile update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::set_avatar(session, &request.mime_type, request.bytes).await {
            Ok(profile) => {
                let profile = map_matrix_own_profile(profile);
                self.send_actions(vec![AppAction::ProfileUpdateSucceeded {
                    request_id: request_id.sequence,
                    profile,
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::ProfileUpdated {
                    request_id,
                    account_key: AccountKey(session.info.user_id.clone()),
                }));
            }
            Err(error) => {
                self.send_actions(vec![AppAction::ProfileUpdateFailed {
                    request_id: request_id.sequence,
                    message: "profile update failed".to_owned(),
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::ProfileOperationFailed {
                        kind: classify_profile_error(&error),
                    },
                );
            }
        }
    }

    /// Non-blocking, cache-first avatar thumbnail handler (Stage R1).
    ///
    /// 1. Cache hit (`Ready`): emit immediately; no SDK call.
    /// 2. Already in-flight: return; the completing task will emit.
    /// 3. Otherwise: insert into `avatar_inflight`, spawn a bounded fetch task
    ///    that posts `AvatarFetched` back into this actor's inbox.
    async fn handle_download_avatar_thumbnail(&mut self, request_id: RequestId, mxc_uri: String) {
        // 1. Cache hit — serve immediately without any I/O.
        if let Some(cached) = self.avatar_cache.get(&mxc_uri) {
            if matches!(cached, AvatarThumbnailState::Ready { .. }) {
                let thumbnail = cached.clone();
                self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
                    mxc_uri: mxc_uri.clone(),
                    thumbnail: thumbnail.clone(),
                }])
                .await;
                self.emit(CoreEvent::Account(
                    AccountEvent::AvatarThumbnailDownloaded {
                        request_id,
                        mxc_uri,
                        thumbnail,
                    },
                ));
                return;
            }
        }

        // 2. Single-flight dedup — a fetch is already running; record this
        //    request_id so the completing task will emit a terminal event for
        //    every waiter, then return without spawning a second task.
        if let Some(waiters) = self.avatar_inflight.get_mut(&mxc_uri) {
            waiters.push(request_id);
            return;
        }

        // 3. No session — emit failure synchronously rather than spawning.
        let Some(session) = self.session.clone() else {
            let thumbnail = AvatarThumbnailState::Failed {
                request_id: request_id.sequence,
                kind: AvatarThumbnailFailureKind::Sdk,
            };
            self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
                mxc_uri: mxc_uri.clone(),
                thumbnail: thumbnail.clone(),
            }])
            .await;
            self.emit(CoreEvent::Account(
                AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    mxc_uri,
                    thumbnail,
                },
            ));
            return;
        };

        // 4. Spawn a bounded fetch task; return immediately.
        // Record the originating request_id as the first waiter.
        self.avatar_inflight
            .insert(mxc_uri.clone(), vec![request_id]);
        let generation = self.avatar_session_generation;
        let semaphore = self.avatar_download_semaphore.clone();
        let tx = self.self_tx.clone();
        let mxc_uri_clone = mxc_uri.clone();

        self.avatar_fetch_tasks.spawn(async move {
            // Acquire a permit before hitting the SDK so at most
            // AVATAR_DOWNLOAD_CONCURRENCY fetches run concurrently.
            let _permit = semaphore.acquire().await;
            let thumbnail = download_avatar_thumbnail(&session, &mxc_uri_clone)
                .await
                .unwrap_or_else(|kind| AvatarThumbnailState::Failed {
                    request_id: request_id.sequence,
                    kind,
                });
            // Best-effort: if the actor is already shut down, the send fails
            // silently — that is correct because the session is gone anyway.
            let _ = tx
                .send(AccountMessage::AvatarFetched {
                    mxc_uri: mxc_uri_clone,
                    generation,
                    thumbnail,
                })
                .await;
        });
    }

    /// Called when a spawned avatar-fetch task completes.  Updates the cache,
    /// removes the in-flight entry, and emits the same outputs as the old
    /// inline path (only the timing changed).
    ///
    /// Fix 1: stale-generation check — if `generation` does not match the
    /// current `avatar_session_generation` the completion belongs to a previous
    /// session; it is silently dropped.
    ///
    /// Fix 2: every waiter in the `avatar_inflight` Vec receives a terminal
    /// `AvatarThumbnailDownloaded` event; only one `AvatarThumbnailUpdated`
    /// action is reduced (one cache write).
    ///
    /// Fix 3: completed/aborted JoinSet entries are reaped non-blockingly at
    /// the start of each call so the JoinSet does not accumulate finished tasks.
    async fn handle_avatar_fetched(
        &mut self,
        mxc_uri: String,
        generation: u64,
        thumbnail: AvatarThumbnailState,
    ) {
        // Fix 3: drain completed tasks non-blockingly so the JoinSet stays
        // bounded.  Only finished entries are removed; no async wait.
        self.reap_avatar_fetch_tasks();

        // Fix 1: drop stale completions from a prior session.
        if generation != self.avatar_session_generation {
            return;
        }

        // Remove and collect all waiting request_ids for this mxc.
        let waiters = self.avatar_inflight.remove(&mxc_uri).unwrap_or_default();

        // Cache the result so subsequent requests for the same URI are served
        // from memory.  Only `Ready` entries are treated as cache hits; failed
        // entries are cached too so repeated requests during a session don't
        // retry immediately, but a future session clear resets them.
        self.avatar_cache.insert(mxc_uri.clone(), thumbnail.clone());

        // Emit one state-delta for the reducer (one cache write, regardless of
        // how many callers were waiting).
        self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
            mxc_uri: mxc_uri.clone(),
            thumbnail: thumbnail.clone(),
        }])
        .await;

        // Fix 2: deliver a terminal event to every waiter. For a Failed
        // thumbnail, rebuild the payload with each waiter's own request_id so
        // the inner AvatarThumbnailState::Failed.request_id matches the outer
        // event request_id (the old inline path produced a per-request payload).
        for request_id in waiters {
            let per_waiter = match &thumbnail {
                AvatarThumbnailState::Failed { kind, .. } => AvatarThumbnailState::Failed {
                    request_id: request_id.sequence,
                    kind: kind.clone(),
                },
                other => other.clone(),
            };
            self.emit(CoreEvent::Account(
                AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    mxc_uri: mxc_uri.clone(),
                    thumbnail: per_waiter,
                },
            ));
        }
    }

    /// Non-blocking reap of completed/aborted avatar-fetch JoinSet entries.
    /// Must not `.await`; called synchronously inside the actor message loop.
    fn reap_avatar_fetch_tasks(&mut self) {
        while self.avatar_fetch_tasks.try_join_next().is_some() {}
    }

    /// Abort all in-flight avatar fetch tasks and clear the per-session cache.
    /// Called on session clear (logout / account switch) and on shutdown.
    ///
    /// Fix 1: increment `avatar_session_generation` so that any `AvatarFetched`
    /// messages that were already queued before the abort are recognised as
    /// stale by `handle_avatar_fetched` and silently dropped.
    fn abort_avatar_fetch_tasks(&mut self) {
        // Replace (drop) the JoinSet rather than only abort_all(): dropping a
        // JoinSet aborts all its tasks AND discards their entries, so cancelled
        // tasks do not linger across repeated request -> session-clear cycles.
        self.avatar_fetch_tasks = tokio::task::JoinSet::new();
        self.avatar_inflight.clear();
        self.avatar_cache.clear();
        clear_renderable_thumbnail_cache();
        // Replace the semaphore so any task that manages to run after abort
        // cannot accidentally re-use a poisoned permit count.
        self.avatar_download_semaphore = Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY));
        // Advance the generation counter so stale completions from tasks that
        // were spawned before this abort are silently rejected.
        self.avatar_session_generation = self.avatar_session_generation.wrapping_add(1);
    }

    async fn handle_ignore_user(&mut self, request_id: RequestId, user_id: String, ignored: bool) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::IgnoredUserUpdateFailed {
                request_id: request_id.sequence,
                user_id: user_id.clone(),
                ignored,
                message: "ignored user update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.send_actions(vec![AppAction::IgnoredUserUpdateRequested {
            request_id: request_id.sequence,
            user_id: user_id.clone(),
            ignored,
        }])
        .await;

        let result = if ignored {
            koushi_sdk::ignore_user(session, &user_id).await
        } else {
            koushi_sdk::unignore_user(session, &user_id).await
        };

        match result {
            Ok(user_ids) => {
                self.send_actions(vec![
                    AppAction::IgnoredUserUpdateSucceeded {
                        request_id: request_id.sequence,
                    },
                    AppAction::IgnoredUsersLoaded {
                        user_ids: user_ids.clone(),
                    },
                ])
                .await;
                let _ = self
                    .timeline_manager
                    .send(TimelineMessage::IgnoredUsersUpdated { user_ids })
                    .await;
            }
            Err(error) => {
                // Reconcile with server state so the optimistic reducer update
                // does not drift after a failure.
                if let Some(action) = ignored_user_ids_action_from_session(session).await {
                    if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
                        let _ = self
                            .timeline_manager
                            .send(TimelineMessage::IgnoredUsersUpdated {
                                user_ids: user_ids.clone(),
                            })
                            .await;
                    }
                    self.send_actions(vec![
                        AppAction::IgnoredUserUpdateFailed {
                            request_id: request_id.sequence,
                            user_id: user_id.clone(),
                            ignored,
                            message: "ignored user update failed".to_owned(),
                        },
                        action,
                    ])
                    .await;
                } else {
                    self.send_actions(vec![AppAction::IgnoredUserUpdateFailed {
                        request_id: request_id.sequence,
                        user_id: user_id.clone(),
                        ignored,
                        message: "ignored user update failed".to_owned(),
                    }])
                    .await;
                }
                self.emit_failure(
                    request_id,
                    CoreFailure::ReportOperationFailed {
                        kind: classify_ignored_user_list_error(&error),
                    },
                );
            }
        }
    }

    async fn handle_report_user(&mut self, request_id: RequestId, user_id: String, reason: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::report_user(session, &user_id, reason).await {
            Ok(()) => {
                self.emit(CoreEvent::Account(AccountEvent::ReportCompleted {
                    request_id,
                    kind: crate::event::ReportKind::User,
                }));
            }
            Err(error) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::ReportOperationFailed {
                        kind: classify_report_error(&error),
                    },
                );
            }
        }
    }

    async fn handle_accept_verification(&mut self, request_id: RequestId, flow_id: u64) {
        let Some(pending) = self
            .verification_request
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
        else {
            self.project_active_or_missing_verification_failure(request_id, flow_id)
                .await;
            return;
        };
        let pending_request_id = pending.request_id;
        let target = pending.target.clone();
        let handle = pending.handle.clone();

        match handle.state() {
            koushi_sdk::MatrixVerificationRequestState::Requested => {
                if let Err(error) = koushi_sdk::accept_verification_request(&handle).await {
                    self.project_verification_failure(
                        flow_id,
                        target,
                        classify_e2ee_trust_error(&error),
                    )
                    .await;
                    return;
                }
                self.project_verification_request_state(pending_request_id, handle.state())
                    .await;
            }
            koushi_sdk::MatrixVerificationRequestState::Ready => {
                match koushi_sdk::start_sas_verification(&handle).await {
                    Ok(Some(sas)) => {
                        self.store_sas_verification(pending_request_id, target, sas)
                            .await;
                    }
                    Ok(None) => {
                        self.project_verification_failure(
                            flow_id,
                            target,
                            TrustOperationFailureKind::Sdk,
                        )
                        .await;
                    }
                    Err(error) => {
                        self.project_verification_failure(
                            flow_id,
                            target,
                            classify_e2ee_trust_error(&error),
                        )
                        .await;
                    }
                }
            }
            koushi_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                self.store_sas_verification(pending_request_id, target, sas)
                    .await;
            }
            koushi_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(pending_request_id)
                    .await;
            }
            koushi_sdk::MatrixVerificationRequestState::Created
            | koushi_sdk::MatrixVerificationRequestState::Cancelled
            | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => {
                self.project_verification_failure(flow_id, target, TrustOperationFailureKind::Sdk)
                    .await;
            }
        }
    }

    async fn handle_confirm_sas_verification(&mut self, request_id: RequestId, flow_id: u64) {
        let Some(pending) = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
        else {
            self.project_active_or_missing_verification_failure(request_id, flow_id)
                .await;
            return;
        };
        let pending_request_id = pending.request_id;
        let target = pending.target.clone();
        let handle = pending.handle.clone();

        match koushi_sdk::confirm_sas_verification(&handle).await {
            Ok(()) => {
                self.project_sas_state(pending_request_id, target, handle.state())
                    .await;
            }
            Err(error) => {
                self.project_verification_failure(
                    flow_id,
                    target,
                    classify_e2ee_trust_error(&error),
                )
                .await;
            }
        }
    }

    async fn handle_cancel_verification(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        reason: VerificationCancelReason,
    ) {
        enum CancelTarget {
            Sas {
                target: VerificationTarget,
                handle: koushi_sdk::MatrixSasVerificationHandle,
            },
            Request {
                target: VerificationTarget,
                handle: koushi_sdk::MatrixVerificationRequestHandle,
            },
        }

        let sas_target = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
            .map(|pending| CancelTarget::Sas {
                target: pending.target.clone(),
                handle: pending.handle.clone(),
            });
        let cancel_target = match reason {
            VerificationCancelReason::Mismatch => sas_target,
            VerificationCancelReason::User => sas_target.or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == flow_id)
                    .map(|pending| CancelTarget::Request {
                        target: pending.target.clone(),
                        handle: pending.handle.clone(),
                    })
            }),
        };

        let Some(cancel_target) = cancel_target else {
            if reason == VerificationCancelReason::Mismatch {
                self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
            } else {
                self.project_active_or_missing_verification_failure(request_id, flow_id)
                    .await;
            }
            return;
        };

        let target = match &cancel_target {
            CancelTarget::Sas { target, .. } | CancelTarget::Request { target, .. } => {
                target.clone()
            }
        };
        let result = match cancel_target {
            CancelTarget::Sas { handle, .. } => match reason {
                VerificationCancelReason::User => {
                    koushi_sdk::cancel_sas_verification(&handle).await
                }
                VerificationCancelReason::Mismatch => {
                    koushi_sdk::mismatch_sas_verification(&handle).await
                }
            },
            CancelTarget::Request { handle, .. } => {
                koushi_sdk::cancel_verification_request(&handle).await
            }
        };

        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;

        if let Err(error) = result {
            self.project_verification_failure(flow_id, target, classify_e2ee_trust_error(&error))
                .await;
        }
    }

    fn observe_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else { break };
                        let terminal = matches!(
                            state,
                            koushi_sdk::MatrixVerificationRequestState::Done
                                | koushi_sdk::MatrixVerificationRequestState::Cancelled
                                | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod
                        );
                        if tx
                            .send(AccountMessage::VerificationRequestProgress {
                                request_id,
                                target: target.clone(),
                                state,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                }
            }
        });
        self.verification_request_observer = Some(VerificationObservation { stop_tx, task });
    }

    fn observe_sas_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixSasVerificationHandle,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else { break };
                        let terminal = matches!(
                            state,
                            koushi_sdk::MatrixSasState::Done
                                | koushi_sdk::MatrixSasState::Cancelled
                                | koushi_sdk::MatrixSasState::UnsupportedShortAuth
                        );
                        if tx
                            .send(AccountMessage::SasVerificationProgress {
                                request_id,
                                target: target.clone(),
                                state,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                }
            }
        });
        self.sas_verification_observer = Some(VerificationObservation { stop_tx, task });
    }

    async fn handle_verification_request_progress(
        &mut self,
        request_id: RequestId,
        _target: VerificationTarget,
        state: koushi_sdk::MatrixVerificationRequestState,
    ) {
        if !self
            .verification_request
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == request_id.sequence)
        {
            return;
        }
        self.project_verification_request_state(request_id, state)
            .await;
    }

    async fn handle_sas_verification_progress(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        state: koushi_sdk::MatrixSasState,
    ) {
        if !self
            .sas_verification
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == request_id.sequence)
        {
            return;
        }
        self.project_sas_state(request_id, target, state).await;
    }

    async fn project_verification_request_state(
        &mut self,
        request_id: RequestId,
        state: koushi_sdk::MatrixVerificationRequestState,
    ) {
        match state {
            koushi_sdk::MatrixVerificationRequestState::Created
            | koushi_sdk::MatrixVerificationRequestState::Requested => {}
            koushi_sdk::MatrixVerificationRequestState::Ready => {
                self.reduce(vec![AppAction::VerificationAccepted {
                    request_id: request_id.sequence,
                }]);
            }
            koushi_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                let Some(target) = self
                    .verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == request_id.sequence)
                    .map(|pending| pending.target.clone())
                else {
                    return;
                };
                self.store_sas_verification(request_id, target, sas).await;
            }
            koushi_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(request_id).await;
            }
            koushi_sdk::MatrixVerificationRequestState::Cancelled => {
                self.project_active_or_missing_verification_failure_with_kind(
                    request_id,
                    request_id.sequence,
                    TrustOperationFailureKind::Cancelled,
                )
                .await;
            }
            koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => {
                self.project_active_or_missing_verification_failure(
                    request_id,
                    request_id.sequence,
                )
                .await;
            }
        }
    }

    async fn store_sas_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixSasVerificationHandle,
    ) {
        self.stop_sas_verification_observer().await;
        self.sas_verification = Some(PendingSasVerification {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.observe_sas_verification(request_id, target.clone(), handle.clone());
        if matches!(handle.state(), koushi_sdk::MatrixSasState::Started)
            && let Err(error) = koushi_sdk::accept_sas_verification(&handle).await
        {
            self.project_verification_failure(
                request_id.sequence,
                target,
                classify_e2ee_trust_error(&error),
            )
            .await;
            return;
        }
        self.project_sas_state(request_id, target, handle.state())
            .await;
    }

    async fn project_sas_state(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        state: koushi_sdk::MatrixSasState,
    ) {
        match state {
            koushi_sdk::MatrixSasState::Created
            | koushi_sdk::MatrixSasState::Started
            | koushi_sdk::MatrixSasState::Accepted => {}
            koushi_sdk::MatrixSasState::SasPresented { emojis } => {
                self.reduce(vec![AppAction::VerificationSasPresented {
                    request_id: request_id.sequence,
                    emojis: emojis.clone(),
                }]);
                self.emit_verification_progress(VerificationFlowState::SasPresented {
                    request_id: request_id.sequence,
                    target,
                    emojis,
                });
            }
            koushi_sdk::MatrixSasState::Confirmed => {}
            koushi_sdk::MatrixSasState::Done => {
                self.project_verification_completed(request_id).await;
            }
            koushi_sdk::MatrixSasState::Cancelled => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    TrustOperationFailureKind::Cancelled,
                )
                .await;
            }
            koushi_sdk::MatrixSasState::UnsupportedShortAuth => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    TrustOperationFailureKind::Sdk,
                )
                .await;
            }
        }
    }

    async fn project_verification_completed(&mut self, request_id: RequestId) {
        let target = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == request_id.sequence)
            .map(|pending| pending.target.clone())
            .or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == request_id.sequence)
                    .map(|pending| pending.target.clone())
            });
        let Some(target) = target else {
            return;
        };

        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;
        self.reduce(vec![AppAction::VerificationCompleted {
            request_id: request_id.sequence,
        }]);
        self.emit_verification_progress(VerificationFlowState::Done {
            request_id: request_id.sequence,
            target,
        });
    }

    async fn project_active_or_missing_verification_failure(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
    ) {
        self.project_active_or_missing_verification_failure_with_kind(
            request_id,
            flow_id,
            TrustOperationFailureKind::Sdk,
        )
        .await;
    }

    async fn project_active_or_missing_verification_failure_with_kind(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        kind: TrustOperationFailureKind,
    ) {
        let target = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
            .map(|pending| pending.target.clone())
            .or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == flow_id)
                    .map(|pending| pending.target.clone())
            });
        match target {
            Some(target) => {
                self.project_verification_failure(flow_id, target, kind)
                    .await
            }
            None => {
                self.reduce(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }]);
                let failure = if self.session.is_some() {
                    CoreFailure::LocalEncryptionUnavailable
                } else {
                    CoreFailure::SessionRequired
                };
                self.emit_failure(request_id, failure);
            }
        }
    }

    async fn project_verification_failure(
        &mut self,
        flow_id: u64,
        target: VerificationTarget,
        kind: TrustOperationFailureKind,
    ) {
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;
        self.reduce(vec![AppAction::VerificationFailed {
            request_id: flow_id,
            kind,
        }]);
        self.emit_verification_progress(VerificationFlowState::Failed {
            request_id: flow_id,
            target,
            kind,
        });
    }

    fn emit_verification_progress(&self, state: VerificationFlowState) {
        if let Some(account_key) = self.active_account_key() {
            self.emit(CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress {
                account_key,
                state,
            }));
        }
    }

    async fn handle_submit_identity_reset_auth(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        request: koushi_state::IdentityResetAuthRequest,
    ) {
        let flow_request_id = RequestId {
            connection_id: request_id.connection_id,
            sequence: flow_id,
        };
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.cancel_identity_reset_handle().await;
                self.reduce(vec![AppAction::ResetIdentityFailed {
                    request_id: flow_id,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = match self.identity_reset_handle.as_ref() {
            Some(handle) => koushi_sdk::complete_identity_reset(&session, handle, &request).await,
            None => Err(koushi_sdk::E2eeTrustError::Sdk(
                "identity reset auth continuation missing".to_owned(),
            )),
        };

        drop(request);

        match result {
            Ok(()) => {
                self.identity_reset_handle = None;
                let (actions, events) =
                    project_reset_identity_completed(flow_request_id, account_key);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(flow_request_id, account_key, error);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
        }
    }

    async fn handle_bootstrap_cross_signing(
        &self,
        request_id: RequestId,
        auth: Option<koushi_state::AuthSecret>,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::bootstrap_cross_signing(&session, auth.as_ref()).await;
        let (actions, events) =
            project_bootstrap_cross_signing_result(request_id, account_key, result);
        self.reduce(actions);
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_enable_key_backup(
        &self,
        request_id: RequestId,
        passphrase: Option<koushi_state::AuthSecret>,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::enable_key_backup(&session, passphrase.as_ref()).await;
        drop(passphrase);
        let (actions, events) = project_enable_key_backup_result(request_id, account_key, result);
        self.reduce(actions);
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_restore_key_backup(
        &self,
        request_id: RequestId,
        version: Option<String>,
        request: RecoveryRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::restore_key_backup(&session, &request, version.as_deref()).await;
        drop(request);

        let (actions, events) = project_restore_key_backup_result(request_id, account_key, result);
        self.reduce(actions);
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_export_room_keys(&self, request_id: RequestId, request: RoomKeyExportRequest) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let RoomKeyExportRequest {
            destination_path,
            passphrase,
        } = request;
        let result =
            koushi_sdk::export_room_keys_to_file(&session, destination_path, &passphrase).await;
        drop(passphrase);
        match result {
            Ok(summary) => {
                self.reduce(vec![AppAction::RoomKeyExported {
                    request_id: request_id.sequence,
                    exported_sessions: summary.exported_sessions,
                }]);
            }
            Err(_) => {
                self.reduce(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_import_room_keys(&self, request_id: RequestId, request: RoomKeyImportRequest) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let RoomKeyImportRequest {
            source_path,
            passphrase,
        } = request;
        let result =
            koushi_sdk::import_room_keys_from_file(&session, source_path, &passphrase).await;
        drop(passphrase);
        match result {
            Ok(summary) => {
                self.reduce(vec![AppAction::RoomKeyImported {
                    request_id: request_id.sequence,
                    imported_count: summary.imported_count,
                    total_count: summary.total_count,
                }]);
            }
            Err(_) => {
                self.reduce(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_bootstrap_secure_backup(
        &self,
        request_id: RequestId,
        request: SecureBackupSetupRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::SecureBackupSetupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let SecureBackupSetupRequest {
            passphrase,
            recovery_key_destination_path,
        } = request;
        let result = koushi_sdk::bootstrap_secure_backup(
            &session,
            passphrase.as_ref(),
            recovery_key_destination_path,
        )
        .await;
        drop(passphrase);
        match result {
            Ok(summary) => {
                let delivery = if summary.recovery_key_written {
                    RecoveryKeyDeliveryState::Written
                } else {
                    RecoveryKeyDeliveryState::NotWritten
                };
                self.reduce(vec![
                    AppAction::SecureBackupRecoveryKeyReady {
                        request_id: request_id.sequence,
                        delivery,
                    },
                    AppAction::SecureBackupSetupEnabled {
                        request_id: request_id.sequence,
                    },
                ]);
            }
            Err(_) => {
                self.reduce(vec![AppAction::SecureBackupSetupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_change_secure_backup_passphrase(
        &self,
        request_id: RequestId,
        request: SecureBackupPassphraseChangeRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::SecureBackupPassphraseChangeFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let SecureBackupPassphraseChangeRequest {
            old_secret,
            new_passphrase,
            recovery_key_destination_path,
        } = request;
        let result = koushi_sdk::change_secure_backup_passphrase(
            &session,
            &old_secret,
            &new_passphrase,
            recovery_key_destination_path,
        )
        .await;
        drop(old_secret);
        drop(new_passphrase);
        match result {
            Ok(summary) => {
                let delivery = if summary.recovery_key_written {
                    RecoveryKeyDeliveryState::Written
                } else {
                    RecoveryKeyDeliveryState::NotWritten
                };
                self.reduce(vec![AppAction::SecureBackupPassphraseChanged {
                    request_id: request_id.sequence,
                    delivery,
                }]);
            }
            Err(_) => {
                self.reduce(vec![AppAction::SecureBackupPassphraseChangeFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_reset_identity(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.cancel_identity_reset_handle().await;
                self.reduce(vec![AppAction::ResetIdentityFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        match koushi_sdk::reset_identity(&session).await {
            Ok(koushi_sdk::IdentityResetOutcome::Completed) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) = project_reset_identity_completed(request_id, account_key);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
            Ok(koushi_sdk::IdentityResetOutcome::AuthRequired(handle)) => {
                let auth_type = handle.desktop_auth_type();
                self.cancel_identity_reset_handle().await;
                self.identity_reset_handle = Some(handle);
                let (actions, events) =
                    project_reset_identity_auth_required(request_id, account_key, auth_type);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(request_id, account_key, error);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
        }
    }

    async fn handle_discover_login(&mut self, _request_id: RequestId, homeserver: String) {
        let requested_homeserver = homeserver.clone();
        let discovery_result =
            tokio::task::spawn_blocking(move || koushi_sdk::discover_login_flows(&homeserver))
                .await;

        match discovery_result {
            Ok(Ok(discovery)) => {
                self.reduce(vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: requested_homeserver,
                    flows: discovery.flows,
                    delegated: discovery.delegated,
                }]);
            }
            Ok(Err(error)) => {
                self.reduce(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver,
                    kind: login_discovery_failure_kind(&error),
                }]);
            }
            Err(_) => {
                self.reduce(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver,
                    kind: AuthFailureKind::Sdk,
                }]);
            }
        }
    }

    async fn handle_start_oidc_login(&mut self, _request_id: RequestId, homeserver: String) {
        self.reduce(vec![AppAction::LoginDiscoveryFailed {
            homeserver,
            kind: AuthFailureKind::Unsupported,
        }]);
    }

    async fn handle_complete_oidc_login(
        &mut self,
        _request_id: RequestId,
        homeserver: String,
        _callback_url: String,
    ) {
        self.reduce(vec![AppAction::LoginDiscoveryFailed {
            homeserver,
            kind: AuthFailureKind::Unsupported,
        }]);
    }

    async fn handle_login_password(&mut self, request_id: RequestId, request: LoginRequest) {
        // Store bootstrap step 1: the password exchange runs on a storeless
        // client. The device id (and therefore the store path) is unknown
        // before this completes. The storeless client must never sync or
        // initialize encryption.
        let login_result = koushi_sdk::login_with_password_with_store(&request, None).await;

        let login_session = match login_result {
            Err(error) => {
                let kind = classify_login_error(&error);
                self.emit_failure(request_id, CoreFailure::LoginFailed { kind });
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
                return;
            }
            Ok(session) => session,
        };

        let info = login_session.info.clone();
        let key_id = session_key_id_from_info(&info);
        let account_key = account_key_from_info(&info);

        // Store bootstrap step 2a: persist the session credentials.
        let persistable = match self.persist_session(&login_session, &key_id) {
            Ok(persistable) => persistable,
            Err(failure) => {
                self.abort_login(login_session, &key_id, false).await;
                self.emit_failure(request_id, failure);
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
                return;
            }
        };

        // Store bootstrap step 2b: restore the session into the per-account
        // encrypted store. The store-backed session replaces the login client
        // BEFORE any sync or E2EE traffic. Fail-closed: if store creation or
        // the store-backed restore fails, the storeless session is dropped,
        // never kept as a fallback.
        let store_backed = match self.restore_into_store(&persistable, &key_id).await {
            Ok(session) => session,
            Err(failure) => {
                self.abort_login(login_session, &key_id, true).await;
                self.emit_failure(request_id, failure);
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
                return;
            }
        };

        // The storeless client never synced; drop it inside the runtime
        // context (Async rule 11).
        drop(login_session);

        let session_arc = Arc::new(store_backed);
        self.device_session_ordinals.clear();
        self.pending_uia_operations.clear();
        self.session = Some(session_arc.clone());
        self.session_key_id = Some(key_id);

        // Spawn the SyncActor now that we have a store-backed session
        // (store bootstrap invariant: sync only on the store-backed session).
        self.spawn_sync_actor(session_arc.clone()).await;

        // Project login success through the reducer (session → Ready), then
        // hydrate Rust-owned profile/account-data projections. Fetch failure is
        // non-fatal to login.
        let mut actions = vec![AppAction::LoginSucceeded(info)];
        if let Some(profile_action) = own_profile_action_from_session(&session_arc).await {
            actions.push(profile_action);
        }
        if let Some(alias_action) = local_user_aliases_action_from_session(&session_arc).await {
            actions.push(alias_action);
        }
        if let Some(action) = ignored_user_ids_action_from_session(&session_arc).await {
            if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
                let _ = self
                    .timeline_manager
                    .send(TimelineMessage::IgnoredUsersUpdated {
                        user_ids: user_ids.clone(),
                    })
                    .await;
            }
            actions.push(action);
        }
        self.reduce(actions);

        // Emit domain event carrying the request_id for command correlation.
        self.emit(CoreEvent::Account(AccountEvent::LoggedIn {
            request_id,
            account_key,
        }));

        // Observe the SDK recovery stream asynchronously. New-device recovery
        // can arrive after login completes, once account data syncs in.
        self.start_recovery_observer(session_arc.clone());
        self.start_incoming_verification_observer(session_arc);
    }

    async fn handle_restore_session(&mut self, request_id: RequestId, account_key: AccountKey) {
        let key_id = match self.lookup_session_key_id(&account_key) {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                // No stored session for this account: project
                // RestoreSessionNotFound so AppState returns to SignedOut, and
                // keep the redacted failure event for command correlation.
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                // Credential store unreachable.
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        self.restore_account(request_id, key_id, RestoreOutcome::Restored)
            .await;
    }

    /// Resolve the last-session pointer inside the actor and run a
    /// store-backed restore. A missing pointer is a NORMAL outcome
    /// (`CoreFailure::SessionNotFound`): the UI goes to login quietly.
    /// A pointer whose session data is missing follows the same not-found
    /// contract (handled inside `restore_account`).
    async fn handle_restore_last_session(&mut self, request_id: RequestId) {
        let key_id = match self.store.credential_backend().load_last_session() {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        self.restore_account(request_id, key_id, RestoreOutcome::Restored)
            .await;
    }

    /// List saved sessions from the credential store. Emits
    /// `AccountEvent::SavedSessionsListed` with identity data only
    /// (homeserver / user_id / device_id) — never tokens or secrets.
    /// An empty list is a normal answer, not a failure.
    fn handle_query_saved_sessions(&self, request_id: RequestId) {
        match self.store.credential_backend().load_saved_sessions() {
            Ok(index) => {
                let sessions = index
                    .sessions()
                    .iter()
                    .map(session_info_from_key_id)
                    .collect();
                self.emit(CoreEvent::Account(AccountEvent::SavedSessionsListed {
                    request_id,
                    sessions,
                }));
            }
            Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            }
        }
    }

    async fn handle_query_devices(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::DeviceSessionsLoadFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        match koushi_sdk::list_devices(&session).await {
            Ok(devices) => {
                let mut ordinal_map = BTreeMap::new();
                let summaries = devices
                    .into_iter()
                    .enumerate()
                    .map(|(index, device)| {
                        let ordinal = index as u64 + 1;
                        ordinal_map.insert(ordinal, device.raw_device_id);
                        DeviceSessionSummary {
                            device_ordinal: ordinal,
                            display_name: device.display_name,
                            current: device.current,
                            verified: device.verified,
                            inactive: device.inactive,
                        }
                    })
                    .collect();
                self.device_session_ordinals = ordinal_map;
                self.reduce(vec![AppAction::DeviceSessionsLoaded {
                    request_id: request_id.sequence,
                    devices: summaries,
                }]);
            }
            Err(_) => {
                self.reduce(vec![AppAction::DeviceSessionsLoadFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            }
        }
    }

    async fn handle_load_account_management_capabilities(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::AccountManagementCapabilitiesLoadFailed]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let capabilities = koushi_sdk::account_management_capabilities(&session).await;
        self.reduce(vec![AppAction::AccountManagementCapabilitiesLoaded {
            change_password: capabilities.change_password,
        }]);
    }

    async fn handle_rename_device(
        &mut self,
        request_id: RequestId,
        device_ordinal: u64,
        display_name: String,
    ) {
        let operation = AccountManagementOperation::RenameDevice;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                );
                return;
            }
        };
        let Some(raw_device_id) = self.device_session_ordinals.get(&device_ordinal).cloned() else {
            self.project_account_management_failure(
                request_id,
                operation,
                AuthFailureKind::Sdk,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Sdk,
                },
            );
            return;
        };

        let result = koushi_sdk::rename_device(&session, &raw_device_id, &display_name).await;
        drop(display_name);
        match result {
            Ok(()) => self.reduce(vec![AppAction::AccountManagementSucceeded {
                request_id: request_id.sequence,
                operation,
            }]),
            Err(_) => self.project_account_management_failure(
                request_id,
                operation,
                AuthFailureKind::Sdk,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Sdk,
                },
            ),
        }
    }

    async fn handle_delete_devices(
        &mut self,
        request_id: RequestId,
        device_ordinals: Vec<u64>,
        auth: Option<koushi_state::IdentityResetAuthRequest>,
    ) {
        let operation = if device_ordinals.len() == 1 {
            AccountManagementOperation::DeleteDevice
        } else {
            AccountManagementOperation::DeleteOtherDevices
        };
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                );
                return;
            }
        };
        let mut raw_device_ids = Vec::with_capacity(device_ordinals.len());
        for ordinal in &device_ordinals {
            let Some(raw_device_id) = self.device_session_ordinals.get(ordinal) else {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
                return;
            };
            raw_device_ids.push(raw_device_id.clone());
        }

        // If this is the first attempt (no auth), try without auth so the
        // server can challenge us with UIA. The challenge response is handled
        // below by projecting AwaitingUia and storing the continuation.
        let uiaa_session = auth
            .as_ref()
            .and_then(|_| self.pending_uia_operations.get(&request_id.sequence))
            .and_then(|pending| pending.uiaa_session.clone());
        let result = koushi_sdk::delete_devices(
            &session,
            &raw_device_ids,
            auth.as_ref(),
            uiaa_session.as_deref(),
        )
        .await;
        drop(auth);
        match result {
            Ok(()) => {
                self.pending_uia_operations.remove(&request_id.sequence);
                self.reduce(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }]);
            }
            Err(koushi_sdk::DeleteDevicesError::UiaaChallenge { session }) => {
                let flow_id = request_id.sequence;
                self.pending_uia_operations.insert(
                    flow_id,
                    PendingUiaOperation {
                        operation,
                        raw_device_ids,
                        new_password: None,
                        erase_data: false,
                        uiaa_session: session,
                    },
                );
                self.reduce(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }]);
            }
            Err(koushi_sdk::DeleteDevicesError::Sdk(_)) => {
                self.pending_uia_operations.remove(&request_id.sequence);
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_change_password(
        &mut self,
        request_id: RequestId,
        new_password: koushi_state::AuthSecret,
    ) {
        let operation = AccountManagementOperation::ChangePassword;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                );
                return;
            }
        };

        let result = koushi_sdk::change_password(&session, &new_password, None, None).await;
        match result {
            Ok(()) => self.reduce(vec![AppAction::AccountManagementSucceeded {
                request_id: request_id.sequence,
                operation,
            }]),
            Err(koushi_sdk::AccountManagementError::UiaaChallenge { session }) => {
                let flow_id = request_id.sequence;
                self.pending_uia_operations.insert(
                    flow_id,
                    PendingUiaOperation {
                        operation,
                        raw_device_ids: Vec::new(),
                        new_password: Some(new_password),
                        erase_data: false,
                        uiaa_session: session,
                    },
                );
                self.reduce(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }]);
            }
            Err(koushi_sdk::AccountManagementError::Sdk(_)) => {
                drop(new_password);
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_deactivate_account(&mut self, request_id: RequestId, erase_data: bool) {
        let operation = AccountManagementOperation::DeactivateAccount;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                );
                return;
            }
        };

        let result = koushi_sdk::deactivate_account(&session, erase_data, None, None).await;
        match result {
            Ok(()) => {
                self.pending_uia_operations.remove(&request_id.sequence);
                self.reduce(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }]);
                // Deactivation ends the account on the server. Perform local
                // sign-out cleanup without sending a second /logout request.
                self.perform_logout(request_id, false).await;
            }
            Err(koushi_sdk::AccountManagementError::UiaaChallenge { session }) => {
                let flow_id = request_id.sequence;
                self.pending_uia_operations.insert(
                    flow_id,
                    PendingUiaOperation {
                        operation,
                        raw_device_ids: Vec::new(),
                        new_password: None,
                        erase_data,
                        uiaa_session: session,
                    },
                );
                self.reduce(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }]);
            }
            Err(koushi_sdk::AccountManagementError::Sdk(_)) => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_submit_account_management_uia(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        auth: koushi_state::IdentityResetAuthRequest,
    ) {
        let Some(mut pending) = self.pending_uia_operations.remove(&flow_id) else {
            self.emit_failure(
                request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Sdk,
                },
            );
            return;
        };
        let operation = pending.operation;
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.project_account_management_failure(
                    RequestId {
                        connection_id: request_id.connection_id,
                        sequence: flow_id,
                    },
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::SessionRequired,
                );
                return;
            }
        };

        let result = match operation {
            AccountManagementOperation::RenameDevice
            | AccountManagementOperation::ThreePid
            | AccountManagementOperation::IdentityServer => {
                // These operations do not use UIA; no pending op should exist.
                self.emit_failure(
                    RequestId {
                        connection_id: request_id.connection_id,
                        sequence: flow_id,
                    },
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
                return;
            }
            AccountManagementOperation::DeleteDevice
            | AccountManagementOperation::DeleteOtherDevices => koushi_sdk::delete_devices(
                &session,
                &pending.raw_device_ids,
                Some(&auth),
                pending.uiaa_session.as_deref(),
            )
            .await
            .map_err(AccountManagementUiaError::DeleteDevices),
            AccountManagementOperation::ChangePassword => {
                let Some(new_password) = pending.new_password.as_ref() else {
                    self.project_account_management_failure(
                        RequestId {
                            connection_id: request_id.connection_id,
                            sequence: flow_id,
                        },
                        operation,
                        AuthFailureKind::Sdk,
                        CoreFailure::AccountOperationFailed {
                            kind: AuthFailureKind::Sdk,
                        },
                    );
                    return;
                };
                koushi_sdk::change_password(
                    &session,
                    new_password,
                    Some(&auth),
                    pending.uiaa_session.as_deref(),
                )
                .await
                .map_err(AccountManagementUiaError::AccountManagement)
            }
            AccountManagementOperation::DeactivateAccount => koushi_sdk::deactivate_account(
                &session,
                pending.erase_data,
                Some(&auth),
                pending.uiaa_session.as_deref(),
            )
            .await
            .map_err(AccountManagementUiaError::AccountManagement),
        };
        drop(auth);
        match result {
            Ok(()) => {
                let was_deactivation = operation == AccountManagementOperation::DeactivateAccount;
                self.reduce(vec![AppAction::AccountManagementSucceeded {
                    request_id: flow_id,
                    operation,
                }]);
                if was_deactivation {
                    self.perform_logout(
                        RequestId {
                            connection_id: request_id.connection_id,
                            sequence: flow_id,
                        },
                        false,
                    )
                    .await;
                }
            }
            Err(AccountManagementUiaError::DeleteDevices(
                koushi_sdk::DeleteDevicesError::UiaaChallenge { session },
            ))
            | Err(AccountManagementUiaError::AccountManagement(
                koushi_sdk::AccountManagementError::UiaaChallenge { session },
            )) => {
                pending.uiaa_session = session;
                self.pending_uia_operations.insert(flow_id, pending);
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Forbidden,
                    },
                );
            }
            Err(AccountManagementUiaError::DeleteDevices(koushi_sdk::DeleteDevicesError::Sdk(
                _,
            )))
            | Err(AccountManagementUiaError::AccountManagement(
                koushi_sdk::AccountManagementError::Sdk(_),
            )) => {
                self.project_account_management_failure(
                    RequestId {
                        connection_id: request_id.connection_id,
                        sequence: flow_id,
                    },
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_soft_logout_reauth(
        &mut self,
        request_id: RequestId,
        password: koushi_state::AuthSecret,
    ) {
        let Some(session) = self.session.as_ref() else {
            self.reduce(vec![AppAction::SoftLogoutReauthFailed {
                request_id: request_id.sequence,
                kind: AuthFailureKind::Sdk,
            }]);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let info = session.info.clone();
        let key_id = session_key_id_from_info(&info);

        // Preserve the existing per-account store (and thus crypto/cached data)
        // by stopping sync and dropping the stale session before re-auth.
        self.stop_sync_actor().await;
        drop(self.session.take());
        self.session_key_id = None;

        let login_session = match koushi_sdk::login_with_existing_device(
            &info.homeserver,
            &info.user_id,
            &info.device_id,
            &password,
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                self.reduce(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }]);
                let failure = CoreFailure::LoginFailed {
                    kind: classify_login_error(&koushi_sdk::PasswordLoginError::Sdk(
                        error.to_string(),
                    )),
                };
                self.emit_failure(request_id, failure);
                return;
            }
        };
        drop(password);

        let persistable = match self.persist_session(&login_session, &key_id) {
            Ok(persistable) => persistable,
            Err(failure) => {
                self.abort_login(login_session, &key_id, false).await;
                self.reduce(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, failure);
                return;
            }
        };

        let store_backed = match self.restore_into_store(&persistable, &key_id).await {
            Ok(session) => session,
            Err(failure) => {
                self.abort_login(login_session, &key_id, true).await;
                self.reduce(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, failure);
                return;
            }
        };
        drop(login_session);

        let session_arc = Arc::new(store_backed);
        self.device_session_ordinals.clear();
        self.pending_uia_operations.clear();
        self.session = Some(session_arc.clone());
        self.session_key_id = Some(key_id);
        self.spawn_sync_actor(session_arc.clone()).await;

        let mut actions = vec![AppAction::SoftLogoutReauthSucceeded {
            request_id: request_id.sequence,
        }];
        if let Some(action) = ignored_user_ids_action_from_session(&session_arc).await {
            if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
                let _ = self
                    .timeline_manager
                    .send(TimelineMessage::IgnoredUsersUpdated {
                        user_ids: user_ids.clone(),
                    })
                    .await;
            }
            actions.push(action);
        }
        self.reduce(actions);

        self.start_recovery_observer(session_arc.clone());
        self.start_incoming_verification_observer(session_arc);
    }

    fn project_account_management_failure(
        &self,
        request_id: RequestId,
        operation: AccountManagementOperation,
        kind: AuthFailureKind,
        failure: CoreFailure,
    ) {
        self.reduce(vec![AppAction::AccountManagementFailed {
            request_id: request_id.sequence,
            operation,
            kind,
        }]);
        self.emit_failure(request_id, failure);
    }

    async fn handle_switch_account(&mut self, request_id: RequestId, account_key: AccountKey) {
        // Ordered shutdown of the current account runtime WITHOUT clearing
        // credentials or stores.
        self.stop_current_session_runtime().await;
        // Drop the SDK handle inside the runtime context (Async rule 11).
        drop(self.session.take());
        self.session_key_id = None;

        let key_id = match self.lookup_session_key_id(&account_key) {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                // Same not-found contract as RestoreSession.
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        // Project the switch intent so the reducer drives state
        // (SwitchingAccount → cleared views), then run the store-backed
        // restore of the target account.
        self.reduce(vec![AppAction::SwitchAccountRequested {
            info: session_info_from_key_id(&key_id),
        }]);

        self.restore_account(request_id, key_id, RestoreOutcome::Switched)
            .await;
    }

    /// Store-backed restore of a known stored account. Shared by
    /// `RestoreSession` and `SwitchAccount`.
    async fn restore_account(
        &mut self,
        request_id: RequestId,
        key_id: SessionKeyId,
        outcome: RestoreOutcome,
    ) {
        let session_json = match self.store.credential_backend().load_matrix_session(&key_id) {
            Ok(stored) => stored,
            Err(err) if koushi_key::is_missing_credential_error(&err) => {
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        let persistable = match PersistableMatrixSession::from_json(session_json.as_str()) {
            Ok(s) => s,
            Err(_) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        match self.restore_into_store(&persistable, &key_id).await {
            Err(failure) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, failure);
            }
            Ok(session) => {
                let info = session.info.clone();
                let account_key = account_key_from_info(&info);

                let session_arc = Arc::new(session);
                self.device_session_ordinals.clear();
                self.pending_uia_operations.clear();
                self.session = Some(session_arc.clone());
                self.session_key_id = Some(key_id);

                // Spawn the SyncActor for the newly restored store-backed session.
                self.spawn_sync_actor(session_arc.clone()).await;
                let mut actions = vec![AppAction::RestoreSessionSucceeded(info)];
                if let Some(profile_action) = own_profile_action_from_session(&session_arc).await {
                    actions.push(profile_action);
                }
                if let Some(alias_action) =
                    local_user_aliases_action_from_session(&session_arc).await
                {
                    actions.push(alias_action);
                }
                if let Some(action) = ignored_user_ids_action_from_session(&session_arc).await {
                    if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
                        let _ = self
                            .timeline_manager
                            .send(TimelineMessage::IgnoredUsersUpdated {
                                user_ids: user_ids.clone(),
                            })
                            .await;
                    }
                    actions.push(action);
                }
                self.reduce(actions);

                self.emit(CoreEvent::Account(match outcome {
                    RestoreOutcome::Restored => AccountEvent::SessionRestored {
                        request_id,
                        account_key,
                    },
                    RestoreOutcome::Switched => AccountEvent::AccountSwitched {
                        request_id,
                        account_key,
                    },
                }));

                self.start_recovery_observer(session_arc.clone());
                self.start_incoming_verification_observer(session_arc);
            }
        }
    }

    /// Submit a recovery secret. Calls the auth crate's `recover_e2ee`
    /// primitive. On success: project E2eeRecoverySucceeded (→ Ready) and emit
    /// RecoveryCompleted. On failure: classify conservatively to
    /// InvalidRecoveryKey/Network/Server (never raw error text) and emit
    /// OperationFailed with RecoveryFailed.
    ///
    /// The recovery secret is NEVER logged, included in error messages, or
    /// stored in any event/snapshot.
    async fn handle_submit_recovery(&mut self, request_id: RequestId, request: RecoveryRequest) {
        let session = match &self.session {
            Some(s) => s.clone(),
            None => {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let account_key = AccountKey(session.info.user_id.clone());

        // Project E2eeRecoverySubmitted so the reducer transitions
        // NeedsRecovery → Recovering while the async call runs.
        self.reduce(vec![AppAction::E2eeRecoverySubmitted(request.clone())]);

        let result = koushi_sdk::recover_e2ee(&session, &request).await;

        // Zero the request secret now — it has been consumed.
        drop(request);

        match result {
            Ok(()) => {
                // Project success: Recovering → Ready.
                self.reduce(vec![AppAction::E2eeRecoverySucceeded]);
                self.reduce(vec![AppAction::RestoreKeyBackupRequested {
                    request_id: request_id.sequence,
                    version: None,
                }]);
                let restore_result =
                    koushi_sdk::download_joined_room_keys_from_backup(&session, None).await;
                let (actions, events) = project_restore_key_backup_result(
                    request_id,
                    account_key.clone(),
                    restore_result,
                );
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
                self.emit(CoreEvent::Account(AccountEvent::RecoveryCompleted {
                    request_id,
                    account_key,
                }));
            }
            Err(error) => {
                let kind = classify_recovery_error(&error);
                // Project failure: Recovering → NeedsRecovery.
                self.reduce(vec![AppAction::E2eeRecoveryFailed {
                    message: "recovery failed".to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::RecoveryFailed { kind });
            }
        }
    }

    async fn perform_logout(&mut self, request_id: RequestId, server_logout: bool) {
        let session = match self.session.take() {
            Some(s) => s,
            None => {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let key_id = self.session_key_id.take();

        self.stop_current_session_runtime().await;

        if server_logout {
            let _ = logout_server_best_effort(&session).await;
        }

        // Drop the SDK handle inside the Tokio runtime context (Async rule 11).
        drop(session);

        // Clean up credentials and stores for this account only.
        let account_key = if let Some(key_id) = &key_id {
            self.clear_account_persistence(key_id);
            AccountKey(key_id.user_id.clone())
        } else {
            AccountKey(String::new())
        };

        self.reduce(vec![AppAction::LogoutFinished]);
        self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
            request_id,
            account_key,
        }));
    }

    async fn handle_logout(&mut self, request_id: RequestId) {
        self.perform_logout(request_id, true).await;
    }

    // --- helpers ---

    /// Persist session credentials, mirroring the src-tauri flow: session
    /// JSON, saved-session index entry, last-session pointer — with rollback
    /// on partial failure.
    fn persist_session(
        &self,
        session: &MatrixClientSession,
        key_id: &SessionKeyId,
    ) -> Result<PersistableMatrixSession, CoreFailure> {
        let backend = self.store.credential_backend();
        let persistable = session
            .persistable_session()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        let json = persistable
            .to_json()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        let stored = StoredMatrixSession::new(json);
        backend
            .save_matrix_session(key_id, &stored)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        if backend.remember_saved_session(key_id).is_err() {
            let _ = backend.delete_matrix_session(key_id);
            return Err(CoreFailure::StoreUnavailable);
        }
        if backend.save_last_session(key_id).is_err() {
            let _ = backend.delete_matrix_session(key_id);
            let _ = backend.forget_saved_session(key_id);
            return Err(CoreFailure::StoreUnavailable);
        }
        Ok(persistable)
    }

    /// Restore a persisted session into the per-account encrypted store
    /// (fail-closed: any store init failure is `LocalEncryptionUnavailable`).
    /// The store config includes the search index so the SDK initializes it
    /// alongside the SQLite store, and event-cache subscription is attempted
    /// before the restored session is returned to any sync/timeline caller.
    /// The encrypted-store diagnostic flag is derived from the keyed store
    /// invariant exposed by `MatrixClientStoreConfig`.
    async fn restore_into_store(
        &self,
        persistable: &PersistableMatrixSession,
        key_id: &SessionKeyId,
    ) -> Result<MatrixClientSession, CoreFailure> {
        let store_config = self.store.account_store_config(key_id)?;
        // Derive the search index configuration. Fail-closed: if the
        // credential store is unreachable, deny the restore (LocalEncryptionUnavailable).
        let search_config = self.store.account_search_index_config(key_id)?;
        let encrypted_store = store_config.store_config.encrypted_at_rest_configured();
        let store_config_with_search = store_config
            .store_config
            .with_search_index_store(search_config.search_index_config);
        let session =
            koushi_sdk::restore_session_with_store(persistable, Some(&store_config_with_search))
                .await
                .map_err(|_| CoreFailure::LocalEncryptionUnavailable)?;
        let event_cache_result = koushi_sdk::enable_event_cache(&session).await;
        self.emit_event_cache_status(encrypted_store, &event_cache_result);
        Ok(session)
    }

    /// Roll back a failed login bootstrap: best-effort server logout of the
    /// storeless client (so no orphan device stays registered), drop it inside
    /// the runtime context, and — if credentials were already persisted —
    /// remove them again so a later restore does not pick up a session whose
    /// token was just invalidated.
    async fn abort_login(
        &self,
        login_session: MatrixClientSession,
        key_id: &SessionKeyId,
        credentials_persisted: bool,
    ) {
        let _ = koushi_sdk::logout(&login_session).await;
        drop(login_session);
        if credentials_persisted {
            self.clear_account_persistence(key_id);
        }
    }

    /// Remove all persisted material for one account: session JSON, saved
    /// session index entry, last-session pointer (only if it points at this
    /// account), unlock secret, and store/cache directories.
    fn clear_account_persistence(&self, key_id: &SessionKeyId) {
        let backend = self.store.credential_backend();
        let _ = backend.delete_matrix_session(key_id);
        let _ = backend.forget_saved_session(key_id);
        match backend.load_last_session() {
            Ok(Some(last)) if last == *key_id => {
                let _ = backend.delete_last_session();
            }
            Ok(_) => {}
            Err(_) => {
                let _ = backend.delete_last_session();
            }
        }
        self.store.delete_account_credentials(key_id);
    }

    /// Find the stored `SessionKeyId` for an account key (the user's Matrix
    /// ID). Checks the last-session pointer first, then the saved-session
    /// index. `Ok(None)` = no stored session; `Err(())` = store unreachable.
    fn lookup_session_key_id(&self, account_key: &AccountKey) -> Result<Option<SessionKeyId>, ()> {
        let backend = self.store.credential_backend();
        match backend.load_last_session() {
            Ok(Some(key_id)) if key_id.user_id == account_key.0 => {
                return Ok(Some(key_id));
            }
            Ok(_) => {}
            Err(_) => return Err(()),
        }
        let index = backend.load_saved_sessions().map_err(|_| ())?;
        Ok(index
            .sessions()
            .iter()
            .find(|session| session.user_id == account_key.0)
            .cloned())
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

    fn emit_event_cache_status(
        &self,
        encrypted_store: bool,
        result: &Result<koushi_sdk::MatrixEventCacheStatus, koushi_sdk::MatrixEventCacheError>,
    ) {
        let (subscribed, subscribe_status, reason_class) = match result {
            Ok(koushi_sdk::MatrixEventCacheStatus::Enabled) => {
                (true, EventCacheSubscribeStatus::Enabled, None)
            }
            Ok(koushi_sdk::MatrixEventCacheStatus::AlreadyEnabled) => {
                (true, EventCacheSubscribeStatus::AlreadyEnabled, None)
            }
            Err(_) => (
                false,
                EventCacheSubscribeStatus::SubscribeFailed,
                Some(EventCacheFailureReasonClass::SubscribeFailed),
            ),
        };
        self.emit(CoreEvent::LocalEncryption(
            LocalEncryptionEvent::EventCacheStatus {
                encrypted_store,
                subscribed,
                subscribe_status,
                reason_class,
            },
        ));
    }

    fn active_account_key(&self) -> Option<AccountKey> {
        self.session
            .as_ref()
            .map(|session| AccountKey(session.info.user_id.clone()))
    }

    fn timeline_event_by_timestamp_request(
        room_id: matrix_sdk::ruma::OwnedRoomId,
        timestamp_ms: u64,
    ) -> matrix_sdk::ruma::api::client::room::get_event_by_timestamp::v1::Request {
        use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, UInt};

        matrix_sdk::ruma::api::client::room::get_event_by_timestamp::v1::Request::since(
            room_id,
            MilliSecondsSinceUnixEpoch(UInt::new_saturating(timestamp_ms)),
        )
    }

    fn handle_probe_local_encryption_health(&self, request_id: RequestId) {
        let health = self
            .session_key_id
            .as_ref()
            .map(|key_id| self.store.probe_local_encryption_health(key_id))
            .unwrap_or(koushi_state::LocalEncryptionHealth::Unknown);
        self.reduce(vec![AppAction::LocalEncryptionHealthChanged {
            request_id: request_id.sequence,
            health,
        }]);
        self.emit(CoreEvent::LocalEncryption(
            LocalEncryptionEvent::HealthChanged { health },
        ));
    }

    async fn handle_reset_local_data(&mut self, request_id: RequestId) {
        let Some(key_id) = self.session_key_id.take() else {
            self.reduce(vec![AppAction::ResetLocalDataFailed {
                request_id: request_id.sequence,
            }]);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.stop_current_session_runtime().await;

        drop(self.session.take());
        self.clear_account_persistence(&key_id);
        self.reduce(vec![
            AppAction::ResetLocalDataCompleted {
                request_id: request_id.sequence,
            },
            AppAction::LogoutFinished,
        ]);
    }

    fn reduce(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.try_send(actions);
    }

    async fn send_actions(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.send(actions).await;
    }
}

fn trace_room_route(stage: &str, command: &RoomCommand) {
    if std::env::var_os("KOUSHI_CORE_ACTOR_TRACE").is_none() {
        return;
    }
    match command {
        RoomCommand::CreateRoom { request_id, .. } => {
            trace_room_route_event(stage, "create_room", *request_id);
        }
        RoomCommand::CreateSpace { request_id, .. } => {
            trace_room_route_event(stage, "create_space", *request_id);
        }
        RoomCommand::SetSpaceChild { request_id, .. } => {
            trace_room_route_event(stage, "set_space_child", *request_id);
        }
        RoomCommand::InviteUser { request_id, .. } => {
            trace_room_route_event(stage, "invite_user", *request_id);
        }
        RoomCommand::AcceptInvite { request_id, .. } => {
            trace_room_route_event(stage, "accept_invite", *request_id);
        }
        _ => {}
    }
}

fn trace_room_route_event(stage: &str, kind: &str, request_id: RequestId) {
    eprintln!(
        "koushi_core actor_trace account_room_route stage={stage} kind={kind} request_id={}/{}",
        request_id.connection_id.0, request_id.sequence
    );
}

fn trace_room_route_closed() {
    if std::env::var_os("KOUSHI_CORE_ACTOR_TRACE").is_some() {
        eprintln!("koushi_core actor_trace account_room_route stage=closed");
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ServerLogoutOutcome {
    Completed,
    Failed,
    TimedOut,
}

async fn logout_server_best_effort(session: &MatrixClientSession) -> ServerLogoutOutcome {
    wait_for_server_logout_best_effort(SERVER_LOGOUT_TIMEOUT, koushi_sdk::logout(session)).await
}

async fn wait_for_server_logout_best_effort<F>(timeout: Duration, request: F) -> ServerLogoutOutcome
where
    F: Future<Output = Result<(), koushi_sdk::PasswordLoginError>>,
{
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(())) => ServerLogoutOutcome::Completed,
        Ok(Err(_)) => ServerLogoutOutcome::Failed,
        Err(_) => ServerLogoutOutcome::TimedOut,
    }
}

fn session_info_from_key_id(key_id: &SessionKeyId) -> SessionInfo {
    SessionInfo {
        homeserver: key_id.homeserver.clone(),
        user_id: key_id.user_id.clone(),
        device_id: key_id.device_id.clone(),
    }
}

async fn run_recovery_state_observation<S>(
    state_stream: S,
    account_key: AccountKey,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut stop_rx: oneshot::Receiver<()>,
) where
    S: futures_util::Stream<Item = E2eeRecoveryState> + Send + 'static,
{
    let mut state_stream = Box::pin(state_stream);
    let mut last_state: Option<E2eeRecoveryState> = None;
    let recovery_methods = vec![RecoveryMethod::RecoveryKey];

    loop {
        let mut pinned_stream = state_stream.as_mut();
        let next_state = pinned_stream.next();
        tokio::select! {
            _ = &mut stop_rx => break,
            state = next_state => {
                let Some(state) = state else {
                    break;
                };
                if last_state == Some(state) {
                    continue;
                }
                last_state = Some(state);

                match state {
                    E2eeRecoveryState::Unknown => {}
                    E2eeRecoveryState::Incomplete => {
                        let _ = action_tx
                            .send(vec![AppAction::E2eeRecoveryStateChanged {
                                state: E2eeRecoveryState::Incomplete,
                                methods: recovery_methods.clone(),
                            }])
                            .await;
                        let _ = event_tx.send(CoreEvent::Account(AccountEvent::RecoveryRequired {
                            account_key: account_key.clone(),
                        }));
                    }
                    E2eeRecoveryState::Enabled | E2eeRecoveryState::Disabled => {
                        let _ = action_tx
                            .send(vec![AppAction::E2eeRecoveryStateChanged {
                                state,
                                methods: recovery_methods.clone(),
                            }])
                            .await;
                    }
                }
            }
        }
    }
}

async fn own_profile_action_from_session(session: &MatrixClientSession) -> Option<AppAction> {
    koushi_sdk::get_own_profile(session)
        .await
        .ok()
        .map(map_matrix_own_profile)
        .map(|profile| AppAction::OwnProfileUpdated { profile })
}

async fn local_user_aliases_action_from_session(
    session: &MatrixClientSession,
) -> Option<AppAction> {
    koushi_sdk::get_local_user_aliases(session)
        .await
        .ok()
        .map(|aliases| AppAction::LocalUserAliasesLoaded {
            aliases: aliases.aliases,
        })
}

async fn ignored_user_ids_action_from_session(session: &MatrixClientSession) -> Option<AppAction> {
    koushi_sdk::get_ignored_user_list(session)
        .await
        .ok()
        .map(|user_ids| AppAction::IgnoredUsersLoaded { user_ids })
}

fn map_matrix_own_profile(profile: koushi_sdk::MatrixOwnProfile) -> OwnProfile {
    OwnProfile {
        display_name: profile.display_name,
        avatar: profile.avatar_mxc_uri.map(|mxc_uri| AvatarImage {
            mxc_uri,
            thumbnail: AvatarThumbnailState::NotRequested,
        }),
    }
}

async fn download_avatar_thumbnail(
    session: &MatrixClientSession,
    mxc_uri: &str,
) -> Result<AvatarThumbnailState, AvatarThumbnailFailureKind> {
    let mxc = <&MxcUri>::from(mxc_uri);
    if !mxc.is_valid() {
        return Err(AvatarThumbnailFailureKind::Unsupported);
    }
    let uri: OwnedMxcUri = mxc.to_owned();
    let bytes = session
        .client()
        .media()
        .get_media_content(
            &MediaRequestParameters {
                source: SdkMediaSource::Plain(uri),
                format: MediaFormat::File,
            },
            false,
        )
        .await
        .map_err(|_| AvatarThumbnailFailureKind::Network)?;

    Ok(store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        mxc_uri,
        bytes,
    ))
}

fn classify_profile_error(error: &koushi_sdk::MatrixProfileError) -> ProfileFailureKind {
    match error.failure_kind() {
        koushi_sdk::MatrixProfileFailureKind::Forbidden => ProfileFailureKind::Forbidden,
        koushi_sdk::MatrixProfileFailureKind::Network => ProfileFailureKind::Network,
        koushi_sdk::MatrixProfileFailureKind::InvalidMimeType => {
            ProfileFailureKind::InvalidMimeType
        }
        koushi_sdk::MatrixProfileFailureKind::Sdk => ProfileFailureKind::Sdk,
    }
}

fn classify_ignored_user_list_error(
    error: &koushi_sdk::MatrixIgnoredUserListError,
) -> crate::failure::ReportFailureKind {
    use crate::failure::ReportFailureKind;
    use koushi_sdk::MatrixIgnoredUserListFailureKind;
    match error.failure_kind() {
        MatrixIgnoredUserListFailureKind::Forbidden => ReportFailureKind::Forbidden,
        MatrixIgnoredUserListFailureKind::Network => ReportFailureKind::Network,
        MatrixIgnoredUserListFailureKind::InvalidUserId => ReportFailureKind::InvalidUserId,
        MatrixIgnoredUserListFailureKind::Sdk => ReportFailureKind::Sdk,
    }
}

fn classify_report_error(
    error: &koushi_sdk::MatrixReportError,
) -> crate::failure::ReportFailureKind {
    use crate::failure::ReportFailureKind;
    use koushi_sdk::MatrixReportFailureKind;
    match error.failure_kind() {
        MatrixReportFailureKind::Forbidden => ReportFailureKind::Forbidden,
        MatrixReportFailureKind::Network => ReportFailureKind::Network,
        MatrixReportFailureKind::InvalidUserId => ReportFailureKind::InvalidUserId,
        MatrixReportFailureKind::InvalidRoomId => ReportFailureKind::InvalidRoomId,
        MatrixReportFailureKind::InvalidEventId => ReportFailureKind::InvalidEventId,
        MatrixReportFailureKind::Sdk => ReportFailureKind::Sdk,
    }
}

/// Map an `E2eeRecoveryError` to a coarse `RecoveryFailureKind` without
/// exposing raw SDK error text in public events or error messages.
/// Conservative classification: prefer InvalidRecoveryKey for auth-type SDK
/// errors, Network for network errors, Server for anything else.
fn classify_recovery_error(
    error: &koushi_sdk::E2eeRecoveryError,
) -> crate::failure::RecoveryFailureKind {
    use crate::failure::RecoveryFailureKind;
    use koushi_sdk::E2eeRecoveryError;
    match error {
        E2eeRecoveryError::Runtime(_) => RecoveryFailureKind::Network,
        E2eeRecoveryError::Sdk(message) => {
            // Classify by error text fragments — these fragments come from the
            // SDK/server and are used only for kind selection, never emitted.
            if message.contains("invalid")
                || message.contains("Invalid")
                || message.contains("M_FORBIDDEN")
                || message.contains("401")
                || message.contains("403")
            {
                RecoveryFailureKind::InvalidRecoveryKey
            } else if message.contains("network")
                || message.contains("timeout")
                || message.contains("connection")
                || message.contains("connect")
            {
                RecoveryFailureKind::Network
            } else {
                RecoveryFailureKind::Server
            }
        }
    }
}

fn classify_e2ee_trust_error(error: &koushi_sdk::E2eeTrustError) -> TrustOperationFailureKind {
    match error {
        koushi_sdk::E2eeTrustError::NoOlmMachine => TrustOperationFailureKind::Sdk,
        koushi_sdk::E2eeTrustError::Sdk(message) => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("timeout") {
                TrustOperationFailureKind::Timeout
            } else if lower.contains("forbidden")
                || lower.contains("m_forbidden")
                || lower.contains("401")
                || lower.contains("403")
            {
                TrustOperationFailureKind::Forbidden
            } else if lower.contains("network")
                || lower.contains("connection")
                || lower.contains("connect")
            {
                TrustOperationFailureKind::Network
            } else {
                TrustOperationFailureKind::Sdk
            }
        }
    }
}

fn project_bootstrap_cross_signing_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<koushi_state::CrossSigningStatus, koushi_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(status) => (
            vec![AppAction::CrossSigningStatusChanged {
                status: status.clone(),
            }],
            vec![CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key,
                status,
            })],
        ),
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = koushi_state::CrossSigningStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_enable_key_backup_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<koushi_state::KeyBackupStatus, koushi_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(koushi_state::KeyBackupStatus::Enabled { version }) => {
            let status = koushi_state::KeyBackupStatus::Enabled {
                version: version.clone(),
            };
            (
                vec![AppAction::KeyBackupEnabled {
                    request_id: request_id.sequence,
                    version,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
        Ok(status) => (
            vec![AppAction::KeyBackupFailed {
                request_id: request_id.sequence,
                kind: TrustOperationFailureKind::Sdk,
            }],
            vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key,
                status,
            })],
        ),
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = koushi_state::KeyBackupStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_restore_key_backup_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<koushi_sdk::KeyBackupRestoreSummary, koushi_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(summary) => {
            let progress_status = koushi_state::KeyBackupStatus::Restoring {
                request_id: request_id.sequence,
                version: summary.version.clone(),
                restored_rooms: summary.restored_rooms,
                total_rooms: summary.total_rooms,
            };
            let restored_status = match summary.version.clone() {
                Some(version) => koushi_state::KeyBackupStatus::Enabled { version },
                None => koushi_state::KeyBackupStatus::Unknown,
            };
            (
                vec![
                    AppAction::KeyBackupRestoreProgress {
                        request_id: request_id.sequence,
                        restored_rooms: summary.restored_rooms,
                        total_rooms: summary.total_rooms,
                    },
                    AppAction::KeyBackupRestored {
                        request_id: request_id.sequence,
                        version: summary.version,
                    },
                ],
                vec![
                    CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                        account_key: account_key.clone(),
                        status: progress_status,
                    }),
                    CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                        account_key,
                        status: restored_status,
                    }),
                ],
            )
        }
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = koushi_state::KeyBackupStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_reset_identity_completed(
    request_id: RequestId,
    account_key: AccountKey,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    (
        vec![AppAction::ResetIdentityCompleted {
            request_id: request_id.sequence,
        }],
        vec![CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state: IdentityResetState::Idle,
        })],
    )
}

fn project_reset_identity_auth_required(
    request_id: RequestId,
    account_key: AccountKey,
    auth_type: IdentityResetAuthType,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let state = IdentityResetState::AwaitingAuth {
        request_id: request_id.sequence,
        auth_type,
    };
    (
        vec![AppAction::ResetIdentityAuthRequired {
            request_id: request_id.sequence,
            auth_type,
        }],
        vec![CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state,
        })],
    )
}

fn project_reset_identity_error(
    request_id: RequestId,
    account_key: AccountKey,
    error: koushi_sdk::E2eeTrustError,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let kind = classify_e2ee_trust_error(&error);
    let state = IdentityResetState::Failed {
        request_id: request_id.sequence,
        kind,
    };
    (
        vec![AppAction::ResetIdentityFailed {
            request_id: request_id.sequence,
            kind,
        }],
        vec![
            CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key: account_key.clone(),
                status: CrossSigningStatus::Failed {
                    request_id: request_id.sequence,
                    kind,
                },
            }),
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged { account_key, state }),
        ],
    )
}

fn incoming_verification_request_id(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(0),
        sequence,
    }
}

/// Map a `PasswordLoginError` to a coarse `LoginFailureKind` without exposing
/// raw SDK error text in public events.
fn classify_login_error(error: &koushi_sdk::PasswordLoginError) -> LoginFailureKind {
    use koushi_sdk::{LoginDiscoveryError, PasswordLoginError};
    match error {
        PasswordLoginError::InvalidHomeserver(discovery_err) => match discovery_err {
            LoginDiscoveryError::RequestFailed(_) | LoginDiscoveryError::HttpStatus { .. } => {
                LoginFailureKind::Network
            }
            _ => LoginFailureKind::Server,
        },
        PasswordLoginError::Sdk(message) => {
            if message.contains("401")
                || message.contains("403")
                || message.contains("M_FORBIDDEN")
                || message.contains("M_UNAUTHORIZED")
            {
                LoginFailureKind::InvalidCredentials
            } else if message.contains("429") || message.contains("M_LIMIT_EXCEEDED") {
                LoginFailureKind::RateLimited
            } else {
                LoginFailureKind::Server
            }
        }
        PasswordLoginError::Runtime(_) => LoginFailureKind::Server,
        PasswordLoginError::MissingSession => LoginFailureKind::Server,
        PasswordLoginError::Serialization(_) => LoginFailureKind::Store,
    }
}

fn login_discovery_failure_kind(error: &koushi_sdk::LoginDiscoveryError) -> AuthFailureKind {
    match error {
        koushi_sdk::LoginDiscoveryError::RequestFailed(_) => AuthFailureKind::Network,
        koushi_sdk::LoginDiscoveryError::HttpStatus { status: 403, .. } => {
            AuthFailureKind::Forbidden
        }
        koushi_sdk::LoginDiscoveryError::HttpStatus { .. }
        | koushi_sdk::LoginDiscoveryError::MissingFlows
        | koushi_sdk::LoginDiscoveryError::InvalidResponse(_) => AuthFailureKind::Sdk,
        koushi_sdk::LoginDiscoveryError::InvalidHomeserver(_)
        | koushi_sdk::LoginDiscoveryError::UnsupportedHomeserverScheme
        | koushi_sdk::LoginDiscoveryError::InsecureHomeserverScheme => AuthFailureKind::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use futures_util::stream;
    use tempfile::tempdir;
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::store::CredentialStoreBackend;

    #[test]
    fn incoming_verification_flow_ids_use_reserved_internal_namespace() {
        let request_id = incoming_verification_request_id(INCOMING_VERIFICATION_FLOW_ID_BASE);

        assert_eq!(request_id.connection_id, RuntimeConnectionId(0));
        assert_eq!(request_id.sequence, INCOMING_VERIFICATION_FLOW_ID_BASE);
    }

    /// Network-free: restoring an account with no stored session must emit the
    /// redacted not-found failure AND project `RestoreSessionNotFound` so the
    /// reducer returns AppState to SignedOut. Same contract for SwitchAccount.
    #[tokio::test]
    async fn restore_and_switch_of_unknown_account_emit_not_found() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );

        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = AccountActor::spawn(store, action_tx, event_tx, LinkPreviewContext::default());

        let request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let account_key = AccountKey("@nobody:example.test".to_owned());

        for command in [
            AccountCommand::RestoreSession {
                request_id,
                account_key: account_key.clone(),
            },
            AccountCommand::SwitchAccount {
                request_id,
                account_key: account_key.clone(),
            },
        ] {
            assert!(handle.send(AccountMessage::Command(command)).await);

            let actions = action_rx.recv().await.expect("reducer actions");
            assert!(
                matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
                "not-found must project RestoreSessionNotFound, got {actions:?}"
            );

            match event_rx.recv().await.expect("event") {
                CoreEvent::OperationFailed {
                    request_id: ev_id,
                    failure,
                } => {
                    assert_eq!(ev_id, request_id);
                    assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
                }
                other => panic!("expected OperationFailed, got {other:?}"),
            }
        }
    }

    fn test_request_id() -> RequestId {
        RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 1,
        }
    }

    #[test]
    fn logout_cleanup_is_bounded_and_ordered_before_persistence_removal() {
        let source = include_str!("account.rs");
        let body = source
            .split("    async fn perform_logout")
            .nth(1)
            .and_then(|rest| rest.split("    async fn handle_logout").next())
            .expect("perform_logout body");

        let shutdown = body
            .find("self.stop_current_session_runtime().await")
            .expect("logout must stop child actors before cleanup");
        let server_logout = body
            .find("logout_server_best_effort(&session).await")
            .expect("server logout must be bounded best-effort");
        let drop_session = body.find("drop(session)").expect("session drop");
        let clear_persistence = body
            .find("self.clear_account_persistence(key_id)")
            .expect("clear account persistence");

        assert!(
            shutdown < server_logout,
            "child actors must release SDK handles before the server logout request"
        );
        assert!(
            server_logout < drop_session,
            "server logout uses the live session but must not replace local cleanup"
        );
        assert!(
            drop_session < clear_persistence,
            "SDK handles must be dropped before deleting local persistence"
        );
        assert!(
            !body.contains("koushi_sdk::logout(&session).await"),
            "logout must not await the network request without a product timeout"
        );
    }

    #[test]
    fn search_crawler_room_notifications_are_latest_wins_and_nonblocking() {
        let source = include_str!("account.rs");
        let notification_arm = source
            .split("AccountMessage::NotifySearchCrawlerRoomsAvailable")
            .nth(1)
            .expect("crawler notification arm")
            .split("AccountMessage::InvalidateSearchCrawlerCache")
            .next()
            .expect("crawler notification arm body");

        assert!(
            notification_arm.contains("self.pending_crawler_notification = Some"),
            "crawler room availability should be stored as a latest-wins notification"
        );
        assert!(
            notification_arm.contains("self.flush_pending_crawler_notification();"),
            "crawler room availability should be flushed without awaiting SearchActor capacity"
        );
        assert!(
            !notification_arm.contains("notify_rooms_available(room_ids, settings).await"),
            "AccountActor must not block user commands on background crawler notification delivery"
        );
    }

    #[test]
    fn restore_into_store_emits_event_cache_status_without_failing_restore() {
        let source = include_str!("account.rs");
        let body = source
            .split("    async fn restore_into_store")
            .nth(1)
            .expect("restore_into_store body")
            .split("    /// Roll back a failed login bootstrap")
            .next()
            .expect("restore_into_store should end before abort_login");
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let helper = source
            .split("    fn emit_event_cache_status(")
            .nth(1)
            .expect("emit_event_cache_status body")
            .split("    fn active_account_key")
            .next()
            .expect("emit_event_cache_status should end before active_account_key");
        let helper_compact: String = helper.chars().filter(|c| !c.is_whitespace()).collect();
        let restore = compact
            .find("koushi_sdk::restore_session_with_store")
            .expect("restore_session_with_store call");
        let store_config = compact
            .find("letstore_config=self.store.account_store_config(key_id)?;")
            .expect("keyed store configuration");
        let encrypted_store = compact
            .find("letencrypted_store=store_config.store_config.encrypted_at_rest_configured();")
            .expect("derived encrypted-store flag");
        let enable = compact
            .find("koushi_sdk::enable_event_cache(&session).await")
            .expect("enable_event_cache call");
        let emit = compact
            .find("self.emit_event_cache_status(encrypted_store,&event_cache_result);")
            .expect("event cache diagnostic emission");
        let return_ok = compact.find("Ok(session)").expect("return statement");

        assert!(store_config < encrypted_store);
        assert!(restore < enable);
        assert!(encrypted_store < emit);
        assert!(enable < return_ok);
        assert!(
            helper_compact.contains("EventCacheSubscribeStatus::Enabled,None"),
            "enabled diagnostics should carry an explicit subscribe status and no failure reason"
        );
        assert!(
            helper_compact.contains("EventCacheSubscribeStatus::AlreadyEnabled,None"),
            "already-enabled diagnostics should carry an explicit subscribe status and no failure reason"
        );
        assert!(
            helper_compact.contains(
                "EventCacheSubscribeStatus::SubscribeFailed,Some(EventCacheFailureReasonClass::SubscribeFailed),",
            ),
            "failure diagnostics should carry an explicit subscribe status and a private-data-free reason"
        );
        assert!(
            compact.contains(
                "letencrypted_store=store_config.store_config.encrypted_at_rest_configured();"
            ),
            "restore_into_store must derive the encrypted-store diagnostic from the keyed store invariant"
        );
        assert!(
            compact.contains("self.emit_event_cache_status(encrypted_store,&event_cache_result);"),
            "restore_into_store must pass the derived encrypted-store flag into the diagnostic"
        );
        assert_eq!(
            compact
                .matches("self.emit_event_cache_status(encrypted_store,&event_cache_result);")
                .count(),
            1,
            "restore_into_store should call the diagnostic helper exactly once"
        );
        assert!(
            !compact.contains("enable_event_cache(&session).await.map_err"),
            "event-cache subscription failure must not be mapped into restore failure"
        );
        assert!(
            !compact.contains("enable_event_cache(&session).await?"),
            "event-cache subscription failure must not use ? to fail the restore path"
        );
        assert!(
            !helper_compact.contains("encrypted_store:true"),
            "the event-cache diagnostic helper must not hardcode the encrypted-store flag"
        );
        assert!(
            !compact.contains("cache_path().is_some()"),
            "restore_into_store must not use cache_path presence as an encryption invariant"
        );
    }

    #[tokio::test]
    async fn server_logout_best_effort_returns_on_timeout() {
        let outcome = wait_for_server_logout_best_effort(
            std::time::Duration::from_millis(1),
            futures_util::future::pending(),
        )
        .await;

        assert_eq!(outcome, ServerLogoutOutcome::TimedOut);
    }

    #[tokio::test]
    async fn server_logout_best_effort_treats_network_failure_as_settled() {
        let outcome =
            wait_for_server_logout_best_effort(std::time::Duration::from_secs(1), async {
                Err(koushi_sdk::PasswordLoginError::Sdk(
                    "synthetic network failure".to_owned(),
                ))
            })
            .await;

        assert_eq!(outcome, ServerLogoutOutcome::Failed);
    }

    fn spawn_actor_with_dirs(
        cred_dir: &std::path::Path,
        data_dir: &std::path::Path,
    ) -> (
        AccountActorHandle,
        mpsc::Receiver<Vec<AppAction>>,
        broadcast::Receiver<CoreEvent>,
    ) {
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(cred_dir)),
            data_dir,
        );
        let (action_tx, action_rx) = mpsc::channel(16);
        let (event_tx, event_rx) = broadcast::channel(16);
        let handle = AccountActor::spawn(store, action_tx, event_tx, LinkPreviewContext::default());
        (handle, action_rx, event_rx)
    }

    /// Network-free: `RestoreLastSession` with no last-session pointer is the
    /// NORMAL first-launch outcome — `SessionNotFound` failure event plus the
    /// `RestoreSessionNotFound` projection so AppState shows SignedOut/login.
    #[tokio::test]
    async fn restore_last_session_without_pointer_emits_not_found() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession { request_id }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("reducer actions");
        assert!(
            matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
            "not-found must project RestoreSessionNotFound, got {actions:?}"
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    /// Network-free: a last-session pointer whose session data is gone (e.g.
    /// cleared by logout) must follow the same not-found contract.
    #[tokio::test]
    async fn restore_last_session_with_dangling_pointer_emits_not_found() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");

        // Seed only the pointer — no session JSON behind it.
        let seeding_backend = CredentialStoreBackend::FileDir(
            crate::store::FileCredentialStore::new(cred_dir.path()),
        );
        let key_id = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@dangling:example.test".to_owned(),
            device_id: "DEVICE1".to_owned(),
        };
        seeding_backend
            .save_last_session(&key_id)
            .expect("seed last-session pointer");

        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession { request_id }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("reducer actions");
        assert!(
            matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
            "dangling pointer must project RestoreSessionNotFound, got {actions:?}"
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    /// Network-free: `QuerySavedSessions` on an empty store answers with an
    /// empty list — a normal outcome, not a failure.
    #[tokio::test]
    async fn query_saved_sessions_empty_store_lists_nothing() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, _action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::QuerySavedSessions { request_id }
                ))
                .await
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: ev_id,
                sessions,
            }) => {
                assert_eq!(ev_id, request_id);
                assert!(sessions.is_empty(), "expected empty list, got {sessions:?}");
            }
            other => panic!("expected SavedSessionsListed, got {other:?}"),
        }
    }

    /// Network-free: `QuerySavedSessions` lists seeded sessions with identity
    /// data only (homeserver / user_id / device_id).
    #[tokio::test]
    async fn query_saved_sessions_lists_seeded_identities() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");

        let seeding_backend = CredentialStoreBackend::FileDir(
            crate::store::FileCredentialStore::new(cred_dir.path()),
        );
        let alpha = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alpha:example.test".to_owned(),
            device_id: "DEVICE-A".to_owned(),
        };
        let beta = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@beta:example.test".to_owned(),
            device_id: "DEVICE-B".to_owned(),
        };
        seeding_backend
            .remember_saved_session(&alpha)
            .expect("seed alpha");
        seeding_backend
            .remember_saved_session(&beta)
            .expect("seed beta");

        let (handle, _action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::QuerySavedSessions { request_id }
                ))
                .await
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: ev_id,
                sessions,
            }) => {
                assert_eq!(ev_id, request_id);
                assert_eq!(sessions.len(), 2);
                assert!(
                    sessions.iter().any(|s| {
                        s.user_id == "@alpha:example.test" && s.device_id == "DEVICE-A"
                    })
                );
                assert!(
                    sessions.iter().any(|s| {
                        s.user_id == "@beta:example.test" && s.device_id == "DEVICE-B"
                    })
                );
                // Identity data only: SessionInfo has exactly homeserver /
                // user_id / device_id (enforced by type); the Debug output of
                // the event must not contain anything token-shaped.
                let debug = format!("{sessions:?}");
                assert!(!debug.contains("access_token"));
                assert!(!debug.contains("secret"));
            }
            other => panic!("expected SavedSessionsListed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reset_local_data_clears_current_account_persistence_and_signs_out_locally() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let key_id = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@reset-user:example.test".to_owned(),
            device_id: "RESETDEVICE".to_owned(),
        };
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );
        let store_config = store
            .account_store_config(&key_id)
            .expect("seed local unlock secret");
        let account_root = store_config
            .store_config
            .path()
            .parent()
            .expect("store path should have account root")
            .to_path_buf();
        std::fs::create_dir_all(store_config.store_config.path()).expect("create store dir");
        std::fs::write(
            store_config.store_config.path().join("sentinel"),
            b"local data",
        )
        .expect("write local store sentinel");
        store
            .credential_backend()
            .save_matrix_session(&key_id, &StoredMatrixSession::new("{\"redacted\":true}"))
            .expect("seed session");
        store
            .credential_backend()
            .remember_saved_session(&key_id)
            .expect("seed saved-session index");
        store
            .credential_backend()
            .save_last_session(&key_id)
            .expect("seed last-session pointer");
        assert_eq!(
            store.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::Healthy
        );

        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _) = broadcast::channel(16);
        let (self_tx, command_rx) = mpsc::channel(16);
        let data_dir_path = store.data_dir().to_path_buf();
        let room_actor = crate::room::RoomActor::spawn(action_tx.clone(), event_tx.clone());
        let messages_backpressure = crate::messages_backpressure::MessagesBackpressure::default();
        let timeline_manager = crate::timeline::TimelineManagerActor::spawn(
            action_tx.clone(),
            event_tx.clone(),
            Some(data_dir_path.clone()),
            messages_backpressure.clone(),
        );
        let mut actor = AccountActor {
            session: None,
            session_key_id: Some(key_id.clone()),
            store,
            action_tx,
            event_tx,
            command_rx,
            self_tx,
            sync_actor: None,
            room_actor,
            timeline_manager,
            messages_backpressure,
            data_dir: data_dir_path,
            link_preview_policy: LinkPreviewContext::default(),
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            device_session_ordinals: BTreeMap::new(),
            pending_uia_operations: BTreeMap::new(),
            verification_request: None,
            sas_verification: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            incoming_verification_observer: None,
            next_incoming_verification_sequence: INCOMING_VERIFICATION_FLOW_ID_BASE,
            pending_crawler_notification: None,
            avatar_cache: HashMap::new(),
            avatar_inflight: HashMap::new(),
            avatar_download_semaphore: Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY)),
            avatar_fetch_tasks: tokio::task::JoinSet::new(),
            avatar_session_generation: 0,
        };
        let request_id = test_request_id();

        actor.handle_reset_local_data(request_id).await;

        let actions = action_rx.recv().await.expect("reset actions");
        assert!(
            matches!(
                actions.as_slice(),
                [
                    AppAction::ResetLocalDataCompleted { request_id: 1 },
                    AppAction::LogoutFinished,
                ]
            ),
            "reset must complete and locally sign out, got {actions:?}"
        );
        assert!(!account_root.exists(), "account root should be removed");

        let check_backend = CredentialStoreBackend::FileDir(
            crate::store::FileCredentialStore::new(cred_dir.path()),
        );
        assert!(koushi_key::is_missing_credential_error(
            &check_backend
                .load_matrix_session(&key_id)
                .expect_err("matrix session should be deleted")
        ));
        assert!(
            check_backend
                .load_saved_sessions()
                .expect("saved-session index")
                .sessions()
                .is_empty()
        );
        assert_eq!(
            check_backend
                .load_last_session()
                .expect("last-session pointer"),
            None
        );
        let check_store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );
        assert_eq!(
            check_store.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::MissingCredential
        );
    }

    /// Recovery-state observation must emit the reducer-legal state change
    /// once per Incomplete transition, even if the stream repeats that state
    /// before later becoming Enabled.
    #[tokio::test]
    async fn recovery_state_observer_deduplicates_repeated_incomplete() {
        let info = SessionInfo {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "DEVICE1".to_owned(),
        };
        let account_key = AccountKey(info.user_id.clone());
        let states = stream::iter([
            koushi_state::E2eeRecoveryState::Unknown,
            koushi_state::E2eeRecoveryState::Incomplete,
            koushi_state::E2eeRecoveryState::Incomplete,
            koushi_state::E2eeRecoveryState::Enabled,
            koushi_state::E2eeRecoveryState::Enabled,
        ]);
        let (action_tx, mut action_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (_stop_tx, stop_rx) = tokio::sync::oneshot::channel();

        run_recovery_state_observation(states, account_key.clone(), action_tx, event_tx, stop_rx)
            .await;

        let first_actions = action_rx.recv().await.expect("first action batch");
        assert_eq!(
            first_actions,
            vec![AppAction::E2eeRecoveryStateChanged {
                state: koushi_state::E2eeRecoveryState::Incomplete,
                methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
            }]
        );

        match event_rx.recv().await.expect("recovery event") {
            CoreEvent::Account(AccountEvent::RecoveryRequired {
                account_key: emitted_key,
            }) => {
                assert_eq!(emitted_key, account_key);
            }
            other => panic!("expected RecoveryRequired event, got {other:?}"),
        }

        let second_actions = action_rx.recv().await.expect("follow-up action batch");
        assert_eq!(
            second_actions,
            vec![AppAction::E2eeRecoveryStateChanged {
                state: koushi_state::E2eeRecoveryState::Enabled,
                methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
            }]
        );

        assert!(
            matches!(
                action_rx.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ),
            "repeated recovery states must not emit duplicate actions"
        );
        assert!(
            matches!(
                event_rx.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ),
            "repeated recovery states must not emit duplicate RecoveryRequired events"
        );
    }

    // -----------------------------------------------------------------------
    // Recovery unit tests (network-free: use fake recovery port)
    // -----------------------------------------------------------------------

    /// Verify classify_recovery_error maps SDK error text to coarse kinds
    /// without leaking the raw message in any public type.
    #[test]
    fn recovery_error_classification_invalid_key() {
        let err = koushi_sdk::E2eeRecoveryError::Sdk("invalid recovery key".to_owned());
        assert_eq!(
            classify_recovery_error(&err),
            crate::failure::RecoveryFailureKind::InvalidRecoveryKey,
            "SDK 'invalid' text must map to InvalidRecoveryKey"
        );
    }

    #[test]
    fn recovery_error_classification_network() {
        let err = koushi_sdk::E2eeRecoveryError::Runtime("runtime error".to_owned());
        assert_eq!(
            classify_recovery_error(&err),
            crate::failure::RecoveryFailureKind::Network,
            "Runtime error must map to Network"
        );
    }

    #[test]
    fn recovery_error_classification_server_fallback() {
        let err = koushi_sdk::E2eeRecoveryError::Sdk("unexpected server error".to_owned());
        assert_eq!(
            classify_recovery_error(&err),
            crate::failure::RecoveryFailureKind::Server,
            "Unknown SDK error must map to Server (conservative)"
        );
    }

    /// Verify that RecoveryRequest's Debug output does not leak the secret.
    #[test]
    fn recovery_request_debug_redacts_secret() {
        use koushi_state::AuthSecret;
        let req = koushi_state::RecoveryRequest {
            secret: AuthSecret::new("super-secret-recovery-key"),
        };
        let debug = format!("{req:?}");
        assert!(
            !debug.contains("super-secret-recovery-key"),
            "RecoveryRequest Debug must redact the secret: {debug}"
        );
    }

    /// Network-free: SubmitRecovery without an active session must emit
    /// SessionRequired, not panic or crash.
    #[tokio::test]
    async fn submit_recovery_without_session_emits_session_required() {
        use koushi_state::AuthSecret;
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, _action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::SubmitRecovery {
                    request_id,
                    request: koushi_state::RecoveryRequest {
                        secret: AuthSecret::new("some-key"),
                    },
                }))
                .await
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
        }
    }

    /// Network-free: E2EE trust commands require an active store-backed
    /// session. Runtime may allow recovery commands while AppState is
    /// NeedsRecovery; without an actor session they must still fail as
    /// SessionRequired, not as local-encryption unavailable.
    #[tokio::test]
    async fn e2ee_trust_commands_without_session_emit_session_required() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::BootstrapCrossSigning {
                        request_id,
                        auth: None,
                    }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("trust failure action batch");
        assert_eq!(
            actions,
            vec![AppAction::BootstrapCrossSigningFailed {
                request_id: request_id.sequence,
                kind: koushi_state::TrustOperationFailureKind::Sdk,
            }]
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn identity_reset_auth_without_session_settles_pending_state() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        let flow_id = 99;
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::SubmitIdentityResetAuth {
                        request_id,
                        flow_id,
                        request: koushi_state::IdentityResetAuthRequest::OAuthApproved,
                    }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("trust failure action batch");
        assert_eq!(
            actions,
            vec![AppAction::ResetIdentityFailed {
                request_id: flow_id,
                kind: koushi_state::TrustOperationFailureKind::Sdk,
            }]
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
        }
    }

    #[test]
    fn e2ee_trust_error_classification_is_kind_only() {
        assert_eq!(
            classify_e2ee_trust_error(&koushi_sdk::E2eeTrustError::NoOlmMachine),
            koushi_state::TrustOperationFailureKind::Sdk
        );
        assert_eq!(
            classify_e2ee_trust_error(&koushi_sdk::E2eeTrustError::Sdk(
                "timeout while talking to @alice:example.test".to_owned()
            )),
            koushi_state::TrustOperationFailureKind::Timeout
        );
        assert_eq!(
            classify_e2ee_trust_error(&koushi_sdk::E2eeTrustError::Sdk("M_FORBIDDEN".to_owned())),
            koushi_state::TrustOperationFailureKind::Forbidden
        );
    }

    #[test]
    fn e2ee_trust_sdk_results_project_actions_and_typed_events() {
        let request_id = test_request_id();
        let account_key = AccountKey("@alice:example.test".to_owned());

        let (actions, events) = project_bootstrap_cross_signing_result(
            request_id,
            account_key.clone(),
            Ok(koushi_state::CrossSigningStatus::Trusted),
        );
        assert_eq!(
            actions,
            vec![AppAction::CrossSigningStatusChanged {
                status: koushi_state::CrossSigningStatus::Trusted,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::CrossSigningChanged {
                    status: koushi_state::CrossSigningStatus::Trusted,
                    ..
                }
            )]
        ));

        let (actions, events) = project_bootstrap_cross_signing_result(
            request_id,
            account_key,
            Err(koushi_sdk::E2eeTrustError::Sdk(
                "timeout from @alice:example.test".to_owned(),
            )),
        );
        assert_eq!(
            actions,
            vec![AppAction::BootstrapCrossSigningFailed {
                request_id: request_id.sequence,
                kind: koushi_state::TrustOperationFailureKind::Timeout,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::CrossSigningChanged {
                    status: koushi_state::CrossSigningStatus::Failed {
                        kind: koushi_state::TrustOperationFailureKind::Timeout,
                        ..
                    },
                    ..
                }
            )]
        ));
        let debug = format!("{events:?}");
        assert!(!debug.contains("@alice:example.test"));
        assert!(!debug.contains("timeout from"));

        let (actions, events) = project_enable_key_backup_result(
            request_id,
            AccountKey("@alice:example.test".to_owned()),
            Ok(koushi_state::KeyBackupStatus::Enabled {
                version: "available".to_owned(),
            }),
        );
        assert_eq!(
            actions,
            vec![AppAction::KeyBackupEnabled {
                request_id: request_id.sequence,
                version: "available".to_owned(),
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::KeyBackupChanged {
                    status: koushi_state::KeyBackupStatus::Enabled { .. },
                    ..
                }
            )]
        ));

        let (actions, events) = project_restore_key_backup_result(
            request_id,
            AccountKey("@alice:example.test".to_owned()),
            Ok(koushi_sdk::KeyBackupRestoreSummary {
                scope: koushi_sdk::KeyBackupRestoreScope::JoinedRooms,
                version: Some("available".to_owned()),
                restored_rooms: 2,
                total_rooms: Some(3),
            }),
        );
        assert_eq!(
            actions,
            vec![
                AppAction::KeyBackupRestoreProgress {
                    request_id: request_id.sequence,
                    restored_rooms: 2,
                    total_rooms: Some(3),
                },
                AppAction::KeyBackupRestored {
                    request_id: request_id.sequence,
                    version: Some("available".to_owned()),
                },
            ]
        );
        assert!(matches!(
            events.as_slice(),
            [
                CoreEvent::E2eeTrust(crate::event::E2eeTrustEvent::KeyBackupChanged {
                    status: koushi_state::KeyBackupStatus::Restoring {
                        restored_rooms: 2,
                        total_rooms: Some(3),
                        ..
                    },
                    ..
                }),
                CoreEvent::E2eeTrust(crate::event::E2eeTrustEvent::KeyBackupChanged {
                    status: koushi_state::KeyBackupStatus::Enabled { .. },
                    ..
                })
            ]
        ));
    }

    #[test]
    fn submit_recovery_hydrates_joined_room_keys_after_secret_recovery() {
        let source = include_str!("account.rs");
        let body = source
            .split("async fn handle_submit_recovery")
            .nth(1)
            .expect("handle_submit_recovery should exist")
            .split("async fn perform_logout")
            .next()
            .expect("perform_logout should follow submit recovery");

        let recover_offset = body
            .find("koushi_sdk::recover_e2ee")
            .expect("submit recovery should recover the secret first");
        let restore_request_offset = body
            .find("AppAction::RestoreKeyBackupRequested")
            .expect("submit recovery should project key backup restore state");
        let restore_offset = body
            .find("koushi_sdk::download_joined_room_keys_from_backup")
            .expect("submit recovery should hydrate joined room keys from backup");

        assert!(recover_offset < restore_request_offset);
        assert!(restore_request_offset < restore_offset);
    }

    #[test]
    fn identity_reset_sdk_results_project_actions_and_typed_events() {
        let request_id = test_request_id();
        let account_key = AccountKey("@alice:example.test".to_owned());

        let (actions, events) = project_reset_identity_completed(request_id, account_key.clone());
        assert_eq!(
            actions,
            vec![AppAction::ResetIdentityCompleted {
                request_id: request_id.sequence,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::IdentityResetChanged {
                    state: koushi_state::IdentityResetState::Idle,
                    ..
                }
            )]
        ));

        let (actions, events) = project_reset_identity_auth_required(
            request_id,
            account_key,
            koushi_state::IdentityResetAuthType::Uiaa,
        );
        assert_eq!(
            actions,
            vec![AppAction::ResetIdentityAuthRequired {
                request_id: request_id.sequence,
                auth_type: koushi_state::IdentityResetAuthType::Uiaa,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::IdentityResetChanged {
                    state: koushi_state::IdentityResetState::AwaitingAuth {
                        auth_type: koushi_state::IdentityResetAuthType::Uiaa,
                        ..
                    },
                    ..
                }
            )]
        ));

        let debug = format!("{events:?}");
        assert!(!debug.contains("@alice:example.test"));
    }
}
