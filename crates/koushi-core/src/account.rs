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
//! the session is restored into the per-account encrypted store without being
//! entered into the active credential index. Only a restricted verification
//! sync may run until authoritative device trust promotes the session; normal
//! sync and persistence start after that promotion. The
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
    collections::{BTreeMap, BTreeSet, HashMap},
    future::Future,
    sync::{Arc, atomic::AtomicU64},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::{SessionKeyId, StoredMatrixSession};
use koushi_sdk::{MatrixClientSession, PendingOidcLogin, PersistableMatrixSession};
use koushi_state::{
    AccountManagementOperation, AppAction, AuthFailureKind, AvatarImage,
    AvatarThumbnailFailureKind, AvatarThumbnailState, CrossSigningStatus, DeviceSessionSummary,
    E2eeRecoveryState, IdentityResetAuthType, IdentityResetState, LoginAttemptId, LoginRequest,
    OperationFailureKind, OwnProfile, PresenceKind, RecoveryKeyDeliveryState, RecoveryMethod,
    RecoveryRequest, SasEmoji, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem,
    SessionInfo, TrustOperationFailureKind, VerificationCancelReason, VerificationFlowState,
    VerificationTarget,
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
use crate::executor;
use crate::failure::{
    CoreFailure, LoginFailureKind, ProfileFailureKind, SyncFailureKind, TimelineFailureKind,
};
use crate::ids::{
    AccountKey, RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey,
    TimelineKind,
};
use crate::link_preview::LinkPreviewContext;
use crate::renderable_thumbnail::{
    RenderableThumbnailKind, clear_renderable_thumbnail_cache, store_renderable_thumbnail,
};
use crate::room::{RoomActorHandle, RoomMessage};
use crate::search::SearchActorHandle;
use crate::startup_trace::{self, StartupPhase};
use crate::store::{StoreActor, account_key_from_info, session_key_id_from_info};
use crate::sync::{SyncActorHandle, SyncMessage};
use crate::timeline::{
    TimelineManagerHandle, TimelineMessage, build_room_message_content_from_composer_body,
};

/// "Credential store healthy, but no stored session for that account"
/// during restore/switch (canon: `CoreFailure::SessionNotFound`).
const SESSION_NOT_FOUND_FAILURE: CoreFailure = CoreFailure::SessionNotFound;

/// Maximum number of concurrent avatar thumbnail downloads. Bounded to avoid
/// flooding the SDK media layer with parallel requests during large room joins.
const AVATAR_DOWNLOAD_CONCURRENCY: usize = 6;
const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";
const SERVER_LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);
const ACCOUNT_HYDRATION_TIMEOUT: Duration = Duration::from_secs(10);
const VERIFICATION_METHOD_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
const IDENTITY_RESET_AUTH_TIMEOUT: Duration = Duration::from_secs(300);
const INCOMING_VERIFICATION_OBSERVER_JOIN_TIMEOUT: Duration = Duration::from_millis(100);
const OIDC_REDIRECT_URI: &str = "koushi-desktop://auth/callback";
/// Redacted message used in reducer error projections (never raw SDK text).
const RESTORE_FAILED_MESSAGE: &str = "session restore failed";
const INCOMING_VERIFICATION_FLOW_ID_BASE: u64 = 1 << 63;

fn scheduled_dispatch_targets_active_session(
    active_session_key: Option<&SessionKeyId>,
    origin_session_key: &SessionKeyId,
) -> bool {
    active_session_key == Some(origin_session_key)
}

fn build_scheduled_message_content(
    body: &str,
    thread_root_event_id: Option<&str>,
) -> Result<matrix_sdk::ruma::events::room::message::RoomMessageEventContent, TimelineFailureKind> {
    use matrix_sdk::ruma::events::{relation::Thread, room::message::Relation};

    let mut content = build_room_message_content_from_composer_body(
        body,
        koushi_state::MentionIntent::default(),
    )?;
    if let Some(root_event_id) = thread_root_event_id {
        let root_event_id = matrix_sdk::ruma::EventId::parse(root_event_id)
            .map_err(|_| TimelineFailureKind::Sdk)?;
        content.relates_to = Some(Relation::Thread(Thread::plain(
            root_event_id.clone(),
            root_event_id,
        )));
    }
    Ok(content)
}

fn state_search_scope(scope: &crate::command::SearchScope) -> koushi_state::SearchScope {
    match scope {
        crate::command::SearchScope::AllRooms => koushi_state::SearchScope::AllRooms,
        crate::command::SearchScope::CurrentRoom { room_id } => {
            koushi_state::SearchScope::CurrentRoom {
                room_id: room_id.clone(),
            }
        }
        crate::command::SearchScope::CurrentSpace { space_id } => {
            koushi_state::SearchScope::CurrentSpace {
                space_id: space_id.clone(),
            }
        }
        crate::command::SearchScope::Dms => koushi_state::SearchScope::Dms,
    }
}

macro_rules! trace_restore {
    ($stage:expr, [$($field:expr),* $(,)?], $($arg:tt)*) => {{
        let event = DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.account",
            $stage,
        )$(.field($field))*;
        record(event);
    }};
}

fn trace_restore_simple(stage: &'static str, action: &'static str) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.account", stage)
            .field(DiagnosticField::token("action", action)),
    );
}

fn record_verification_admission_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn verification_admission_event(
    stage: &'static str,
    generation: u64,
    transition_id: u64,
) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.verification_admission", stage)
        .field(DiagnosticField::count("generation", generation))
        .field(DiagnosticField::count("transition_id", transition_id))
}

fn current_device_trust_token(trust: koushi_state::CurrentDeviceTrustState) -> &'static str {
    match trust {
        koushi_state::CurrentDeviceTrustState::Unknown => "unknown",
        koushi_state::CurrentDeviceTrustState::Unverified => "unverified",
        koushi_state::CurrentDeviceTrustState::Verified => "verified",
    }
}

fn record_verification_method_discovery_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn verification_method_discovery_event(
    stage: &'static str,
    generation: u64,
    serial: u64,
) -> DiagnosticEvent {
    DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.verification_method_discovery",
        stage,
    )
    .field(DiagnosticField::count("generation", generation))
    .field(DiagnosticField::count("serial", serial))
}

fn trace_account_request(stage: &'static str, request_id: RequestId, action: &'static str) {
    trace_restore!(
        stage,
        [
            DiagnosticField::token("action", action),
            DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence
            ),
        ],
        "request_id={} action={}",
        request_id_trace_label(request_id),
        action
    );
}

/// Messages routed to the AccountActor task.
pub enum AccountMessage {
    Command(AccountCommand),
    SyncCommand(SyncCommand),
    RoomCommand(RoomCommand),
    TimelineCommand(TimelineCommand),
    ResolveActivity {
        generation: u64,
        requests: Vec<crate::activity_resolution::ActivityResolutionRequest>,
    },
    CancelActivityResolution,
    AcknowledgeTimelineProjection {
        projection_request_id: RequestId,
        key: TimelineKey,
        generation: TimelineGeneration,
        response: oneshot::Sender<bool>,
    },
    AcknowledgeTimelineBatchRendered {
        key: TimelineKey,
        actor_generation: u64,
        timeline_generation: TimelineGeneration,
        repair_generation: u64,
        batch_id: TimelineBatchId,
    },
    ScheduleServerDelayedSend {
        request_id: RequestId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        send_at_ms: u64,
    },
    DispatchLocalScheduledSend {
        request_id: RequestId,
        origin_session_key: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
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
        thread_root_event_id: Option<String>,
        body: String,
        delay_id: String,
        send_at_ms: u64,
    },
    OpenTimelineAtTimestamp {
        request_id: RequestId,
        room_id: String,
        timestamp_ms: u64,
    },
    EnsureRoomEventCached {
        request_id: RequestId,
        room_id: String,
        event_id: String,
        response_tx: oneshot::Sender<()>,
    },
    RepairRoomTimeline {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
    },
    SearchCommand(SearchCommand),
    /// Record `AppEffect::NotifySearchCrawlerRoomsAvailable` as a latest-wins
    /// background crawler notification and try to flush it to SearchActor.
    NotifySearchCrawlerRoomsAvailable {
        room_ids: Vec<String>,
        settings: koushi_state::SearchCrawlerSettings,
    },
    CurrentDeviceTrustChanged {
        generation: u64,
        trust: koushi_state::CurrentDeviceTrustState,
    },
    FirstRestrictedSyncFinished {
        generation: u64,
        succeeded: bool,
    },
    RestrictedSyncSucceeded {
        generation: u64,
    },
    RestrictedSyncFailed {
        generation: u64,
    },
    VerificationMethodsDiscovered {
        generation: u64,
        serial: u64,
        result: VerificationMethodDiscoveryResult,
    },
    RecoveryFinished {
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
        result: Result<(), koushi_sdk::E2eeRecoveryError>,
    },
    TrustProjectionApplied {
        generation: u64,
        transition_id: u64,
        ready: bool,
        locked: bool,
    },
    RejectProvisionalSession {
        request_id: RequestId,
    },
    RetrySessionTeardown {
        generation: u64,
    },
    #[cfg(test)]
    AttachLifecycleProbe {
        probe_tx: mpsc::UnboundedSender<&'static str>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ConfigureTrustObservation {
        observation: koushi_sdk::CurrentDeviceTrustObservation,
    },
    #[cfg(test)]
    InspectSessionRuntime {
        response: oneshot::Sender<(bool, bool, bool, bool)>,
    },
    #[cfg(test)]
    InspectSyncOwners {
        response: oneshot::Sender<(bool, bool, bool)>,
    },
    #[cfg(test)]
    ConfigureSyntheticRecoveryTask {
        flow_id: u64,
        pending: bool,
    },
    #[cfg(test)]
    ConfigureRecoveryDownload {
        completion: oneshot::Receiver<bool>,
    },
    #[cfg(test)]
    InspectRecoveryTask {
        response: oneshot::Sender<bool>,
    },
    #[cfg(test)]
    ConfigureSyntheticVerification {
        flow_id: u64,
    },
    #[cfg(test)]
    SettleSyntheticVerification {
        flow_id: u64,
        terminal: SyntheticVerificationTerminal,
    },
    #[cfg(test)]
    InspectVerificationRuntime {
        response: oneshot::Sender<(bool, bool, bool, bool, bool, bool, bool)>,
    },
    #[cfg(test)]
    ConfigureOidcCompletion {
        start_request_id: RequestId,
        homeserver: String,
        session: MatrixClientSession,
    },
    #[cfg(test)]
    ConfigureCloseStoreResults {
        results: Vec<bool>,
    },
    #[cfg(test)]
    ShutdownWithAck {
        acknowledged: oneshot::Sender<()>,
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
    SasVerificationTimedOut {
        flow_id: u64,
    },
    VerificationRequestObserverEnded {
        flow_id: u64,
    },
    SasVerificationObserverEnded {
        flow_id: u64,
    },
    IncomingVerificationRequest {
        generation: u64,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    },
    SessionInvalidated {
        soft_logout: bool,
    },
    IdentityResetAuthTimedOut {
        flow_id: u64,
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
    /// Internal: optional account-data/profile hydration completed after the
    /// session was already projected as ready. Generation-gated so stale
    /// completions from a previous session are dropped.
    AccountHydrationLoaded {
        generation: u64,
        actions: Vec<AppAction>,
        ignored_user_ids: Option<BTreeSet<String>>,
    },
    Shutdown,
}

/// Handle to the AccountActor background task.
#[derive(Clone)]
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
    observer: koushi_sdk::MatrixIncomingVerificationRequestObserver,
}

async fn send_incoming_verification_message_until_stopped<T>(
    sender: &mpsc::Sender<T>,
    message: T,
    stop_rx: &mut oneshot::Receiver<()>,
) -> bool {
    tokio::select! {
        biased;
        _ = stop_rx => false,
        result = sender.send(message) => result.is_ok(),
    }
}

async fn stop_incoming_verification_observation(observation: IncomingVerificationObservation) {
    stop_incoming_verification_observation_with_timeout(
        observation,
        INCOMING_VERIFICATION_OBSERVER_JOIN_TIMEOUT,
    )
    .await;
}

async fn stop_incoming_verification_observation_with_timeout(
    observation: IncomingVerificationObservation,
    timeout: Duration,
) {
    let IncomingVerificationObservation {
        stop_tx,
        mut task,
        mut observer,
    } = observation;
    let _ = stop_tx.send(());
    if executor::timeout(timeout, &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
    observer.shutdown().await;
}

struct SessionChangeObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct PendingVerificationRequest {
    request_id: RequestId,
    target: VerificationTarget,
    handle: koushi_sdk::MatrixVerificationRequestHandle,
}

struct PendingRecoveryTask {
    generation: u64,
    flow_id: u64,
    request_id: RequestId,
    task: crate::executor::JoinHandle<()>,
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

struct PendingSessionTeardown {
    generation: u64,
    attempt: u32,
    session: Arc<MatrixClientSession>,
    key_id: Option<SessionKeyId>,
    continuation: SessionTeardownContinuation,
}

enum SessionTeardownContinuation {
    Logout {
        request_id: RequestId,
        server_logout: bool,
    },
    InstallReplacement {
        session: MatrixClientSession,
        persistable: PersistableMatrixSession,
        key_id: SessionKeyId,
        action: AppAction,
    },
}

enum PendingOidcFlow {
    Sdk(PendingOidcLogin),
    #[cfg(test)]
    Synthetic {
        homeserver: String,
    },
}

impl PendingOidcFlow {
    fn homeserver(&self) -> &str {
        match self {
            Self::Sdk(pending) => pending.homeserver(),
            #[cfg(test)]
            Self::Synthetic { homeserver } => homeserver,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrustLifecycleDecision {
    IgnoreStale,
    StayGated,
    Promote,
    Lock,
    AlreadyReady,
}

#[derive(Clone, Copy, Debug)]
struct PendingTrustTransition {
    generation: u64,
    transition_id: u64,
    decision: TrustLifecycleDecision,
}

struct OwnedVerificationMethodDiscoveryTask {
    generation: u64,
    serial: u64,
    task: crate::executor::JoinHandle<()>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationMethodDiscoveryResult {
    Discovered(koushi_state::VerificationGateState),
    Failed(koushi_state::VerificationGateFailureKind),
}

fn trust_projection_ack_matches(
    pending: &PendingTrustTransition,
    generation: u64,
    transition_id: u64,
    ready: bool,
    locked: bool,
) -> bool {
    pending.generation == generation
        && pending.transition_id == transition_id
        && match pending.decision {
            TrustLifecycleDecision::Promote => ready && !locked,
            TrustLifecycleDecision::Lock => locked && !ready,
            _ => false,
        }
}

#[derive(Clone, Copy)]
enum VerificationTerminal {
    Success,
    Cancelled(VerificationCancelReason),
    Failed(TrustOperationFailureKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SasAdoptionDecision {
    Adopt,
    Replay,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IncomingVerificationRequestDecision {
    Adopt,
    Replay,
    Conflict,
}

#[derive(Clone, Copy)]
struct IncomingVerificationActivity<'a> {
    active_request: Option<(&'a VerificationTarget, &'a str)>,
    sas_active: bool,
    own_user_active: bool,
}

fn classify_incoming_verification_request(
    activity: IncomingVerificationActivity<'_>,
    incoming_target: &VerificationTarget,
    incoming_flow_id: &str,
) -> IncomingVerificationRequestDecision {
    if activity.own_user_active {
        return IncomingVerificationRequestDecision::Conflict;
    }

    match activity.active_request {
        Some((active_target, active_flow_id))
            if active_target == incoming_target && active_flow_id == incoming_flow_id =>
        {
            IncomingVerificationRequestDecision::Replay
        }
        Some(_) => IncomingVerificationRequestDecision::Conflict,
        None if activity.sas_active => IncomingVerificationRequestDecision::Conflict,
        None => IncomingVerificationRequestDecision::Adopt,
    }
}

fn incoming_verification_request_is_current(
    message_generation: u64,
    current_generation: u64,
    has_session: bool,
) -> bool {
    has_session && message_generation == current_generation
}

fn classify_sas_adoption(
    active_flow_id: Option<u64>,
    incoming_flow_id: u64,
) -> SasAdoptionDecision {
    match active_flow_id {
        None => SasAdoptionDecision::Adopt,
        Some(active_flow_id) if active_flow_id == incoming_flow_id => SasAdoptionDecision::Replay,
        Some(_) => SasAdoptionDecision::Conflict,
    }
}

async fn resolve_sas_adoption<F, Fut>(
    active_flow_id: Option<u64>,
    incoming_flow_id: u64,
    reject_conflict: F,
) -> (SasAdoptionDecision, Option<bool>)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = bool>,
{
    let decision = classify_sas_adoption(active_flow_id, incoming_flow_id);
    let rejection_succeeded = match decision {
        SasAdoptionDecision::Conflict => Some(reject_conflict().await),
        SasAdoptionDecision::Adopt | SasAdoptionDecision::Replay => None,
    };
    (decision, rejection_succeeded)
}

fn sas_verification_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.sas_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
}

fn record_sas_verification_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn verification_request_state_token(
    state: &koushi_sdk::MatrixVerificationRequestState,
) -> &'static str {
    match state {
        koushi_sdk::MatrixVerificationRequestState::Created => "created",
        koushi_sdk::MatrixVerificationRequestState::Requested => "requested",
        koushi_sdk::MatrixVerificationRequestState::Ready => "ready",
        koushi_sdk::MatrixVerificationRequestState::SasStarted(_) => "sas_started",
        koushi_sdk::MatrixVerificationRequestState::Done => "done",
        koushi_sdk::MatrixVerificationRequestState::Cancelled { .. } => "cancelled",
        koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => "unsupported_method",
    }
}

fn verification_cancel_kind_token(kind: koushi_sdk::MatrixVerificationCancelKind) -> &'static str {
    match kind {
        koushi_sdk::MatrixVerificationCancelKind::UnknownMethod => "unknown_method",
        koushi_sdk::MatrixVerificationCancelKind::KeyMismatch => "key_mismatch",
        koushi_sdk::MatrixVerificationCancelKind::User => "user",
        koushi_sdk::MatrixVerificationCancelKind::Timeout => "timeout",
        koushi_sdk::MatrixVerificationCancelKind::AcceptedElsewhere => "accepted_elsewhere",
        koushi_sdk::MatrixVerificationCancelKind::Other => "other",
    }
}

fn sas_state_token(state: &koushi_sdk::MatrixSasState) -> &'static str {
    match state {
        koushi_sdk::MatrixSasState::Created => "created",
        koushi_sdk::MatrixSasState::Started => "started",
        koushi_sdk::MatrixSasState::Accepted => "accepted",
        koushi_sdk::MatrixSasState::SasPresented { .. } => "sas_presented",
        koushi_sdk::MatrixSasState::Confirmed => "confirmed",
        koushi_sdk::MatrixSasState::Done => "done",
        koushi_sdk::MatrixSasState::Cancelled { .. } => "cancelled",
        koushi_sdk::MatrixSasState::UnsupportedShortAuth => "unsupported_short_auth",
    }
}

fn sas_state_changed_event(flow_id: u64, state: &koushi_sdk::MatrixSasState) -> DiagnosticEvent {
    let mut event = sas_verification_event("sas_state_changed", flow_id)
        .field(DiagnosticField::token("state", sas_state_token(state)));
    if let koushi_sdk::MatrixSasState::Cancelled {
        kind,
        cancelled_by_us,
    } = state
    {
        event = event
            .field(DiagnosticField::token(
                "cancel_kind",
                verification_cancel_kind_token(*kind),
            ))
            .field(DiagnosticField::boolean(
                "cancelled_by_us",
                *cancelled_by_us,
            ));
    }
    event
}

fn trust_failure_token(kind: TrustOperationFailureKind) -> &'static str {
    match kind {
        TrustOperationFailureKind::Cancelled => "cancelled",
        TrustOperationFailureKind::Mismatch => "mismatch",
        TrustOperationFailureKind::InvalidPassphrase => "invalid_passphrase",
        TrustOperationFailureKind::Network => "network",
        TrustOperationFailureKind::Forbidden => "forbidden",
        TrustOperationFailureKind::Timeout => "timeout",
        TrustOperationFailureKind::Sdk => "sdk",
    }
}

fn verification_gate_failure_token(
    kind: koushi_state::VerificationGateFailureKind,
) -> &'static str {
    match kind {
        koushi_state::VerificationGateFailureKind::Network => "network",
        koushi_state::VerificationGateFailureKind::Cancelled => "cancelled",
        koushi_state::VerificationGateFailureKind::Mismatch => "mismatch",
        koushi_state::VerificationGateFailureKind::Forbidden => "forbidden",
        koushi_state::VerificationGateFailureKind::Timeout => "timeout",
        koushi_state::VerificationGateFailureKind::Sdk => "sdk",
        koushi_state::VerificationGateFailureKind::NoProofMethod => "no_proof_method",
    }
}

fn verification_terminal_token(terminal: VerificationTerminal) -> &'static str {
    match terminal {
        VerificationTerminal::Success => "success",
        VerificationTerminal::Cancelled(_) => "cancelled",
        VerificationTerminal::Failed(_) => "failed",
    }
}

fn verification_cancel_reason_token(reason: VerificationCancelReason) -> &'static str {
    match reason {
        VerificationCancelReason::User => "user",
        VerificationCancelReason::Mismatch => "mismatch",
    }
}

async fn run_own_user_sas_start<T, F>(
    flow_id: u64,
    source: &'static str,
    start: F,
) -> Result<Option<T>, koushi_sdk::E2eeTrustError>
where
    F: Future<Output = Result<Option<T>, koushi_sdk::E2eeTrustError>>,
{
    record_sas_verification_event(
        sas_verification_event("sas_start_attempted", flow_id)
            .field(DiagnosticField::token("source", source)),
    );
    let result = start.await;
    let mut event = sas_verification_event("sas_start_finished", flow_id)
        .field(DiagnosticField::token("source", source));
    event = match &result {
        Ok(Some(_)) => event.field(DiagnosticField::token("outcome", "started")),
        Ok(None) => event.field(DiagnosticField::token("outcome", "pending")),
        Err(error) => {
            let kind = classify_e2ee_trust_error(error);
            event
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::token(
                    "failure_kind",
                    trust_failure_token(kind),
                ))
        }
    };
    record_sas_verification_event(event);
    result
}

fn should_report_restricted_sync_failure(failure_reported: &mut bool, succeeded: bool) -> bool {
    if succeeded {
        *failure_reported = false;
        false
    } else if *failure_reported {
        false
    } else {
        *failure_reported = true;
        true
    }
}

fn restricted_sync_blocks_sync_once(restricted_sync_active: bool, command: &SyncCommand) -> bool {
    restricted_sync_active && matches!(command, SyncCommand::SyncOnce { .. })
}

fn begin_restricted_sync_cursor_attempt(restricted_sync_active: bool) -> bool {
    !restricted_sync_active
}

#[cfg(feature = "qa-bin")]
async fn refresh_device_keys_and_assert_known(
    session: &MatrixClientSession,
    target: VerificationTarget,
) -> Result<(), ()> {
    let user_id = matrix_sdk::ruma::UserId::parse(target.user_id).map_err(|_| ())?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(target.device_id);
    let encryption = session.client().encryption();

    let _ = encryption
        .request_user_identity(&user_id)
        .await
        .map_err(|_| ())?;
    encryption
        .get_device(&user_id, &device_id)
        .await
        .map_err(|_| ())?
        .ok_or(())?;
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum SyntheticVerificationTerminal {
    Success,
    Cancelled(VerificationCancelReason),
    Failed(TrustOperationFailureKind),
}

fn trust_lifecycle_decision(
    generation: u64,
    active_generation: u64,
    promoted: bool,
    trust: koushi_state::CurrentDeviceTrustState,
) -> TrustLifecycleDecision {
    if generation != active_generation {
        TrustLifecycleDecision::IgnoreStale
    } else if promoted {
        if matches!(trust, koushi_state::CurrentDeviceTrustState::Verified) {
            TrustLifecycleDecision::AlreadyReady
        } else {
            TrustLifecycleDecision::Lock
        }
    } else if matches!(trust, koushi_state::CurrentDeviceTrustState::Verified) {
        TrustLifecycleDecision::Promote
    } else {
        TrustLifecycleDecision::StayGated
    }
}

/// The account actor's internal state.
pub struct AccountActor {
    /// Active store-backed session, if any.
    session: Option<Arc<MatrixClientSession>>,
    /// Session key for credential store operations.
    session_key_id: Option<SessionKeyId>,
    provisional_persistable: Option<PersistableMatrixSession>,
    session_promoted: bool,
    trust_generation: u64,
    trust_observer: Option<crate::executor::JoinHandle<()>>,
    verification_method_discovery_task: Option<OwnedVerificationMethodDiscoveryTask>,
    verification_method_discovery_serial: u64,
    verification_method_discovery_failed: bool,
    recovery_task: Option<PendingRecoveryTask>,
    restricted_sync: Option<crate::executor::JoinHandle<()>>,
    pending_ready_events: Vec<CoreEvent>,
    pending_trust_transition: Option<PendingTrustTransition>,
    next_trust_transition_id: u64,
    pending_session_teardown: Option<PendingSessionTeardown>,
    next_teardown_generation: u64,
    teardown_retry_task: Option<crate::executor::JoinHandle<()>>,
    #[cfg(test)]
    lifecycle_probe: Option<mpsc::UnboundedSender<&'static str>>,
    #[cfg(any(test, feature = "test-hooks"))]
    trust_observation_override: std::sync::Mutex<Option<koushi_sdk::CurrentDeviceTrustObservation>>,
    #[cfg(any(test, feature = "test-hooks"))]
    trust_observation_is_synthetic: bool,
    #[cfg(test)]
    recovery_download_override: std::sync::Mutex<Option<oneshot::Receiver<bool>>>,
    #[cfg(test)]
    close_store_results: std::collections::VecDeque<bool>,
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
    /// Monotonic across SyncActor replacement so lifecycle projections from a
    /// restarted actor cannot be rejected behind the previous actor's fence.
    sync_generation: Arc<AtomicU64>,
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
    activity_resolution_task: Option<crate::executor::JoinHandle<()>>,
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
    /// Flow id for the pending SDK identity reset continuation.
    identity_reset_flow_id: Option<u64>,
    /// Timeout task for the pending identity reset auth challenge.
    identity_reset_timeout_task: Option<crate::executor::JoinHandle<()>>,
    /// Actor-private mapping from app-owned device ordinal to raw Matrix
    /// device id. Raw ids never enter reducer state or snapshots.
    device_session_ordinals: BTreeMap<u64, String>,
    /// Pending UIA operations keyed by the flow id (original request id).
    /// Holds the data needed to retry a destructive action after the user
    /// supplies interactive auth. Secrets (password, UIA session) are held
    /// only inside this actor-private map, never in reducer state.
    pending_uia_operations: BTreeMap<u64, PendingUiaOperation>,
    /// Pending OAuth authorization-code flow, keyed by originating request id.
    /// Holds SDK client, PKCE verifier, and CSRF validation data inside Rust.
    pending_oidc_login: Option<(RequestId, PendingOidcFlow)>,
    #[cfg(test)]
    oidc_completion_override: Option<MatrixClientSession>,
    /// Pending SDK verification request continuation, held only inside
    /// AccountActor and never projected into AppState.
    verification_request: Option<PendingVerificationRequest>,
    /// Pending SDK SAS continuation, held only inside AccountActor and never
    /// projected into AppState.
    sas_verification: Option<PendingSasVerification>,
    own_user_verification: Option<(u64, koushi_sdk::MatrixOwnUserVerificationHandle)>,
    /// SDK verification request observer task for the active flow.
    verification_request_observer: Option<VerificationObservation>,
    /// SDK SAS observer task for the active flow.
    sas_verification_observer: Option<VerificationObservation>,
    sas_timeout_task: Option<crate::executor::JoinHandle<()>>,
    #[cfg(test)]
    synthetic_verification: Option<(u64, VerificationTarget)>,
    /// SDK incoming verification request observer for the active session.
    incoming_verification_observer: Option<IncomingVerificationObservation>,
    /// Epoch attached to incoming verification messages from the active SDK client.
    incoming_verification_session_generation: u64,
    /// SDK session-change observer for auth invalidation / soft logout.
    session_change_observer: Option<SessionChangeObservation>,
    /// Optional profile/account-data hydration task for the active session.
    account_hydration_task: Option<crate::executor::JoinHandle<()>>,
    /// Incremented whenever optional account hydration is invalidated.
    account_hydration_generation: u64,
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
            provisional_persistable: None,
            session_promoted: false,
            trust_generation: 0,
            trust_observer: None,
            verification_method_discovery_task: None,
            verification_method_discovery_serial: 0,
            verification_method_discovery_failed: false,
            recovery_task: None,
            restricted_sync: None,
            pending_ready_events: Vec::new(),
            pending_trust_transition: None,
            next_trust_transition_id: 0,
            pending_session_teardown: None,
            next_teardown_generation: 0,
            teardown_retry_task: None,
            #[cfg(test)]
            lifecycle_probe: None,
            #[cfg(any(test, feature = "test-hooks"))]
            trust_observation_override: std::sync::Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            trust_observation_is_synthetic: false,
            #[cfg(test)]
            recovery_download_override: std::sync::Mutex::new(None),
            #[cfg(test)]
            close_store_results: std::collections::VecDeque::new(),
            store: store_actor,
            action_tx,
            event_tx,
            command_rx,
            self_tx: tx.clone(),
            sync_actor: None,
            sync_generation: Arc::new(AtomicU64::new(0)),
            room_actor,
            timeline_manager,
            messages_backpressure,
            activity_resolution_task: None,
            data_dir,
            link_preview_policy: initial_link_preview_policy,
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            identity_reset_flow_id: None,
            identity_reset_timeout_task: None,
            device_session_ordinals: BTreeMap::new(),
            pending_uia_operations: BTreeMap::new(),
            pending_oidc_login: None,
            #[cfg(test)]
            oidc_completion_override: None,
            verification_request: None,
            sas_verification: None,
            own_user_verification: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            sas_timeout_task: None,
            #[cfg(test)]
            synthetic_verification: None,
            incoming_verification_observer: None,
            incoming_verification_session_generation: 0,
            session_change_observer: None,
            account_hydration_task: None,
            account_hydration_generation: 0,
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
        #[cfg(test)]
        let mut shutdown_ack = None;
        while let Some(msg) = self.command_rx.recv().await {
            match msg {
                AccountMessage::Shutdown => break,
                #[cfg(test)]
                AccountMessage::ShutdownWithAck { acknowledged } => {
                    shutdown_ack = Some(acknowledged);
                    break;
                }
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
                AccountMessage::ResolveActivity {
                    generation,
                    requests,
                } => {
                    if let Some(task) = self.activity_resolution_task.take() {
                        task.abort();
                    }
                    let Some(session) = self.session.clone() else {
                        let _ = self
                            .action_tx
                            .send(vec![AppAction::ActivityResolutionFailed {
                                generation,
                                unresolved_room_count: requests
                                    .len()
                                    .try_into()
                                    .unwrap_or(u32::MAX),
                                kind: OperationFailureKind::Sdk,
                            }])
                            .await;
                        continue;
                    };
                    let action_tx = self.action_tx.clone();
                    let backpressure = self.messages_backpressure.clone();
                    self.activity_resolution_task = Some(crate::executor::spawn(async move {
                        let outcome = crate::activity_resolution::resolve_activity_requests(
                            &session,
                            &requests,
                            &backpressure,
                        )
                        .await;
                        let settlement = match outcome.failure_kind {
                            Some(kind) => AppAction::ActivityResolutionFailed {
                                generation,
                                unresolved_room_count: outcome.unresolved_room_count,
                                kind,
                            },
                            None => AppAction::ActivityResolutionSucceeded { generation },
                        };
                        let _ = action_tx
                            .send(vec![
                                AppAction::ActivityResolutionRowsObserved {
                                    generation,
                                    rows: outcome.rows,
                                },
                                settlement,
                            ])
                            .await;
                    }));
                }
                AccountMessage::CancelActivityResolution => {
                    if let Some(task) = self.activity_resolution_task.take() {
                        task.abort();
                    }
                }
                AccountMessage::AcknowledgeTimelineProjection {
                    projection_request_id,
                    key,
                    generation,
                    response,
                } => {
                    if !self
                        .timeline_manager
                        .send(TimelineMessage::AcknowledgeProjection {
                            projection_request_id,
                            key,
                            generation,
                            response,
                        })
                        .await
                    {
                        // Dropping the response sender settles the caller as rejected.
                    }
                }
                AccountMessage::AcknowledgeTimelineBatchRendered {
                    key,
                    actor_generation,
                    timeline_generation,
                    repair_generation,
                    batch_id,
                } => {
                    let _ = self
                        .timeline_manager
                        .send(TimelineMessage::AcknowledgeBatchRendered {
                            key,
                            actor_generation,
                            timeline_generation,
                            repair_generation,
                            batch_id,
                        })
                        .await;
                }
                AccountMessage::ScheduleServerDelayedSend {
                    request_id,
                    scheduled_id,
                    room_id,
                    thread_root_event_id,
                    body,
                    send_at_ms,
                } => {
                    self.handle_schedule_server_delayed_send(
                        request_id,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
                        send_at_ms,
                    )
                    .await;
                }
                AccountMessage::DispatchLocalScheduledSend {
                    request_id,
                    origin_session_key,
                    scheduled_id,
                    room_id,
                    thread_root_event_id,
                    body,
                } => {
                    self.handle_dispatch_local_scheduled_send(
                        request_id,
                        origin_session_key,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
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
                    thread_root_event_id,
                    body,
                    delay_id,
                    send_at_ms,
                } => {
                    self.handle_reschedule_server_delayed_send(
                        request_id,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
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
                AccountMessage::EnsureRoomEventCached {
                    request_id,
                    room_id,
                    event_id,
                    response_tx,
                } => {
                    self.handle_ensure_room_event_cached(request_id, room_id, event_id)
                        .await;
                    let _ = response_tx.send(());
                }
                AccountMessage::RepairRoomTimeline {
                    request_id,
                    account_key,
                    room_id,
                } => {
                    self.route_timeline_command(TimelineCommand::RepairGaps {
                        request_id,
                        key: TimelineKey {
                            account_key,
                            kind: TimelineKind::Room { room_id },
                        },
                    })
                    .await;
                }
                AccountMessage::SearchCommand(search_command) => {
                    self.route_search_command(search_command).await;
                }
                AccountMessage::NotifySearchCrawlerRoomsAvailable { room_ids, settings } => {
                    // Background lane: crawler room availability is
                    // latest-wins/coalesced/recoverable state. Store it first,
                    // then try a non-blocking flush so AccountActor never stalls
                    // user-intent or foreground room/timeline commands behind
                    // crawler mailbox pressure.
                    self.pending_crawler_notification = Some((room_ids, settings));
                    self.flush_pending_crawler_notification();
                }
                AccountMessage::CurrentDeviceTrustChanged { generation, trust } => {
                    self.handle_current_device_trust(generation, trust).await;
                }
                AccountMessage::FirstRestrictedSyncFinished {
                    generation,
                    succeeded,
                } => {
                    if first_restricted_sync_is_current(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                    ) {
                        if succeeded {
                            self.discover_verification_methods(generation).await;
                        } else {
                            self.send_actions(vec![AppAction::VerificationMethodsDiscovered(
                                unknown_verification_gate(),
                            )])
                            .await;
                        }
                    }
                }
                AccountMessage::RestrictedSyncSucceeded { generation } => {
                    let own_flow_id = self
                        .own_user_verification
                        .as_ref()
                        .map(|(flow_id, _)| *flow_id);
                    if let Some(flow_id) = active_own_user_sas_flow_for_restricted_sync(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                        own_flow_id,
                    ) {
                        record_sas_verification_event(sas_verification_event(
                            "restricted_sync_succeeded",
                            flow_id,
                        ));
                    }
                    let eligible = own_user_sas_recheck_is_current(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                        self.own_user_verification.is_some(),
                        self.sas_verification.is_some(),
                    );
                    if eligible {
                        self.recheck_own_user_sas_after_sync().await;
                    }
                }
                AccountMessage::RestrictedSyncFailed { generation } => {
                    let own_flow_id = self
                        .own_user_verification
                        .as_ref()
                        .map(|(flow_id, _)| *flow_id);
                    if let Some(flow_id) = active_own_user_sas_flow_for_restricted_sync(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                        own_flow_id,
                    ) {
                        record_sas_verification_event(sas_verification_event(
                            "restricted_sync_failed",
                            flow_id,
                        ));
                    }
                }
                AccountMessage::VerificationMethodsDiscovered {
                    generation,
                    serial,
                    result,
                } => {
                    let outcome = match &result {
                        VerificationMethodDiscoveryResult::Discovered(_) => "success",
                        VerificationMethodDiscoveryResult::Failed(_) => "failed",
                    };
                    record_verification_method_discovery_event(
                        verification_method_discovery_event(
                            "completion_received",
                            generation,
                            serial,
                        )
                        .field(DiagnosticField::token("outcome", outcome)),
                    );
                    let owned_matches = self
                        .verification_method_discovery_task
                        .as_ref()
                        .is_some_and(|owned| {
                            owned.generation == generation && owned.serial == serial
                        });
                    if method_discovery_is_current(
                        generation,
                        self.trust_generation,
                        serial,
                        self.verification_method_discovery_serial,
                        self.session.is_some(),
                    ) && owned_matches
                    {
                        if let Some(owned) = self.verification_method_discovery_task.take() {
                            let _ = owned.task.await;
                        }
                        match result {
                            VerificationMethodDiscoveryResult::Discovered(gate) => {
                                self.verification_method_discovery_failed = false;
                                self.send_actions(vec![AppAction::VerificationMethodsDiscovered(
                                    gate,
                                )])
                                .await;
                            }
                            VerificationMethodDiscoveryResult::Failed(kind) => {
                                self.verification_method_discovery_failed = true;
                                record_verification_method_discovery_event(
                                    verification_method_discovery_event(
                                        "failure_projected",
                                        generation,
                                        serial,
                                    )
                                    .field(
                                        DiagnosticField::token(
                                            "failure_kind",
                                            verification_gate_failure_token(kind),
                                        ),
                                    ),
                                );
                                self.send_actions(vec![
                                    AppAction::VerificationMethodDiscoveryFailed {
                                        generation,
                                        kind,
                                    },
                                ])
                                .await;
                            }
                        }
                    } else {
                        record_verification_method_discovery_event(
                            verification_method_discovery_event(
                                "completion_ignored",
                                generation,
                                serial,
                            ),
                        );
                    }
                }
                AccountMessage::RecoveryFinished {
                    generation,
                    flow_id,
                    request_id,
                    result,
                } => {
                    self.handle_recovery_finished(generation, flow_id, request_id, result)
                        .await;
                }
                AccountMessage::TrustProjectionApplied {
                    generation,
                    transition_id,
                    ready,
                    locked,
                } => {
                    self.handle_trust_projection_applied(generation, transition_id, ready, locked)
                        .await;
                }
                AccountMessage::RejectProvisionalSession { request_id } => {
                    self.perform_logout(request_id, true).await;
                }
                AccountMessage::RetrySessionTeardown { generation } => {
                    self.retry_session_teardown(generation).await;
                }
                #[cfg(test)]
                AccountMessage::AttachLifecycleProbe { probe_tx } => {
                    self.lifecycle_probe = Some(probe_tx);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ConfigureTrustObservation { observation } => {
                    *self
                        .trust_observation_override
                        .lock()
                        .expect("trust observation override lock") = Some(observation);
                }
                #[cfg(test)]
                AccountMessage::InspectSessionRuntime { response } => {
                    let _ = response.send((
                        self.session.is_some(),
                        self.session_promoted,
                        self.sync_actor.is_some(),
                        self.trust_observer.is_some(),
                    ));
                }
                #[cfg(test)]
                AccountMessage::InspectSyncOwners { response } => {
                    let _ = response.send((
                        self.restricted_sync.is_some(),
                        false,
                        self.sync_actor.is_some(),
                    ));
                }
                #[cfg(test)]
                AccountMessage::ConfigureSyntheticRecoveryTask { flow_id, pending } => {
                    self.stop_recovery_task().await;
                    let request_id = incoming_verification_request_id(flow_id);
                    self.recovery_task = Some(PendingRecoveryTask {
                        generation: self.trust_generation,
                        flow_id,
                        request_id,
                        task: if pending {
                            crate::executor::spawn(std::future::pending())
                        } else {
                            crate::executor::spawn(async {})
                        },
                    });
                }
                #[cfg(test)]
                AccountMessage::ConfigureRecoveryDownload { completion } => {
                    *self
                        .recovery_download_override
                        .lock()
                        .expect("recovery download lock") = Some(completion);
                }
                #[cfg(test)]
                AccountMessage::InspectRecoveryTask { response } => {
                    let _ = response.send(self.recovery_task.is_some());
                }
                #[cfg(test)]
                AccountMessage::ConfigureSyntheticVerification { flow_id } => {
                    self.synthetic_verification = Some((
                        flow_id,
                        VerificationTarget {
                            user_id: "@self:example.test".to_owned(),
                            device_id: "DEVICE".to_owned(),
                        },
                    ));
                    let (request_stop, request_stopped) = oneshot::channel();
                    self.verification_request_observer = Some(VerificationObservation {
                        stop_tx: request_stop,
                        task: executor::spawn(async move {
                            let _ = request_stopped.await;
                        }),
                    });
                    let (sas_stop, sas_stopped) = oneshot::channel();
                    self.sas_verification_observer = Some(VerificationObservation {
                        stop_tx: sas_stop,
                        task: executor::spawn(async move {
                            let _ = sas_stopped.await;
                        }),
                    });
                    self.sas_timeout_task = Some(executor::spawn(std::future::pending()));
                }
                #[cfg(test)]
                AccountMessage::SettleSyntheticVerification { flow_id, terminal } => {
                    let terminal = match terminal {
                        SyntheticVerificationTerminal::Success => VerificationTerminal::Success,
                        SyntheticVerificationTerminal::Cancelled(reason) => {
                            VerificationTerminal::Cancelled(reason)
                        }
                        SyntheticVerificationTerminal::Failed(kind) => {
                            VerificationTerminal::Failed(kind)
                        }
                    };
                    self.settle_verification(flow_id, terminal).await;
                }
                #[cfg(test)]
                AccountMessage::InspectVerificationRuntime { response } => {
                    let _ = response.send((
                        self.verification_request.is_some(),
                        self.sas_verification.is_some(),
                        self.own_user_verification.is_some(),
                        self.verification_request_observer.is_some(),
                        self.sas_verification_observer.is_some(),
                        self.sas_timeout_task.is_some(),
                        self.synthetic_verification.is_some(),
                    ));
                }
                #[cfg(test)]
                AccountMessage::ConfigureOidcCompletion {
                    start_request_id,
                    homeserver,
                    session,
                } => {
                    self.pending_oidc_login =
                        Some((start_request_id, PendingOidcFlow::Synthetic { homeserver }));
                    self.oidc_completion_override = Some(session);
                }
                #[cfg(test)]
                AccountMessage::ConfigureCloseStoreResults { results } => {
                    self.close_store_results = results.into();
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
                AccountMessage::SasVerificationTimedOut { flow_id } => {
                    self.handle_sas_verification_timeout(flow_id).await;
                }
                AccountMessage::VerificationRequestObserverEnded { flow_id } => {
                    if self.active_verification_target(flow_id).is_some() {
                        record_sas_verification_event(
                            sas_verification_event("observer_ended", flow_id)
                                .field(DiagnosticField::token("observer", "request")),
                        );
                        self.settle_verification(
                            flow_id,
                            VerificationTerminal::Failed(TrustOperationFailureKind::Sdk),
                        )
                        .await;
                    }
                }
                AccountMessage::SasVerificationObserverEnded { flow_id } => {
                    if self.active_verification_target(flow_id).is_some() {
                        record_sas_verification_event(
                            sas_verification_event("observer_ended", flow_id)
                                .field(DiagnosticField::token("observer", "sas")),
                        );
                        self.settle_verification(
                            flow_id,
                            VerificationTerminal::Failed(TrustOperationFailureKind::Sdk),
                        )
                        .await;
                    }
                }
                AccountMessage::IncomingVerificationRequest {
                    generation,
                    target,
                    handle,
                } => {
                    if incoming_verification_request_is_current(
                        generation,
                        self.incoming_verification_session_generation,
                        self.session.is_some(),
                    ) {
                        let request_id = self.next_incoming_verification_request_id();
                        self.handle_incoming_verification_request(request_id, target, handle)
                            .await;
                    }
                }
                AccountMessage::SessionInvalidated { soft_logout } => {
                    self.handle_session_invalidated(soft_logout).await;
                }
                AccountMessage::IdentityResetAuthTimedOut { flow_id } => {
                    self.handle_identity_reset_auth_timeout(flow_id).await;
                }
                AccountMessage::AvatarFetched {
                    mxc_uri,
                    generation,
                    thumbnail,
                } => {
                    self.handle_avatar_fetched(mxc_uri, generation, thumbnail)
                        .await;
                }
                AccountMessage::AccountHydrationLoaded {
                    generation,
                    actions,
                    ignored_user_ids,
                } => {
                    self.handle_account_hydration_loaded(generation, actions, ignored_user_ids)
                        .await;
                }
            }
            self.flush_pending_crawler_notification();
        }
        self.shutdown_owned_runtime().await;
        self.stop_room_actor().await;
        #[cfg(test)]
        if let Some(acknowledged) = shutdown_ack {
            let _ = acknowledged.send(());
        }
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
        let query_context = match &command {
            SearchCommand::Query {
                request_id,
                query,
                scope,
            } => Some((*request_id, query.clone(), scope.clone())),
            _ => None,
        };
        match &self.search_actor {
            Some(handle) => {
                if !handle.send_command(command).await {
                    if let Some((request_id, query, scope)) = query_context.as_ref() {
                        self.emit_search_failed(
                            *request_id,
                            query,
                            scope,
                            SEARCH_UNAVAILABLE_MESSAGE,
                        )
                        .await;
                    }
                    self.emit_failure(request_id, CoreFailure::SessionRequired);
                }
            }
            None => {
                if let Some((request_id, query, scope)) = query_context.as_ref() {
                    self.emit_search_failed(*request_id, query, scope, SEARCH_UNAVAILABLE_MESSAGE)
                        .await;
                }
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
        }
    }

    async fn shutdown_owned_runtime(&mut self) {
        if let Some(task) = self.teardown_retry_task.take() {
            task.abort();
            let _ = task.await;
            self.record_lifecycle_probe("teardown_retry_terminated");
        }
        self.stop_current_session_runtime().await;
        if let Some(session) = self.session.take() {
            let _ = koushi_sdk::close_session_stores(&session).await;
            drop(session);
            self.record_lifecycle_probe("current_session_released");
        }
        if let Some(pending) = self.pending_session_teardown.take() {
            let _ = koushi_sdk::close_session_stores(&pending.session).await;
            drop(pending.session);
            if let SessionTeardownContinuation::InstallReplacement { session, .. } =
                pending.continuation
            {
                let _ = koushi_sdk::close_session_stores(&session).await;
                drop(session);
            }
            self.record_lifecycle_probe("pending_teardown_sessions_released");
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
        thread_root_event_id: Option<String>,
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
                .send_server_delayed_message(
                    session,
                    &room_id,
                    thread_root_event_id.as_deref(),
                    &body,
                    send_at_ms,
                )
                .await
            {
                Ok(delay_id) => {
                    self.send_actions(vec![
                        AppAction::ScheduledSendCapabilityChanged {
                            capability: ScheduledSendCapability::ServerDelayedEvents,
                        },
                        AppAction::ScheduledSendCreated {
                            item: ScheduledSendItem {
                                scheduled_id,
                                room_id,
                                thread_root_event_id,
                                body,
                                send_at_ms,
                                handle: ScheduledSendHandle::Server { delay_id },
                                is_dispatching: false,
                            },
                        },
                    ])
                    .await;
                    return;
                }
                Err(()) => {}
            }
        }

        self.send_actions(vec![
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
            AppAction::ScheduledSendCreated {
                item: ScheduledSendItem {
                    scheduled_id,
                    room_id,
                    thread_root_event_id,
                    body,
                    send_at_ms,
                    handle: ScheduledSendHandle::Local,
                    is_dispatching: false,
                },
            },
        ])
        .await;
    }

    async fn handle_dispatch_local_scheduled_send(
        &self,
        request_id: RequestId,
        origin_session_key: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
    ) {
        let retry_at_ms = crate::scheduled_send::local_scheduled_send_retry_at_ms();
        if !scheduled_dispatch_targets_active_session(
            self.session_key_id.as_ref(),
            &origin_session_key,
        ) {
            self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                .await;
            return;
        }
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                .await;
            return;
        };
        let room_id = match matrix_sdk::ruma::RoomId::parse(&room_id) {
            Ok(room_id) => room_id,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                    .await;
                return;
            }
        };
        let Some(room) = session.client().get_room(&room_id) else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                .await;
            return;
        };
        let content = match build_scheduled_message_content(&body, thread_root_event_id.as_deref())
        {
            Ok(content) => content,
            Err(kind) => {
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                    .await;
                return;
            }
        };
        let transaction_id = matrix_sdk::ruma::OwnedTransactionId::from(
            crate::scheduled_send::scheduled_send_transaction_id(&scheduled_id),
        );
        match room.send(content).with_transaction_id(transaction_id).await {
            Ok(_) => {
                self.send_actions(vec![AppAction::ScheduledSendDispatched { scheduled_id }])
                    .await;
            }
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                self.retry_local_scheduled_send(scheduled_id, retry_at_ms)
                    .await;
            }
        }
    }

    async fn retry_local_scheduled_send(&self, scheduled_id: String, retry_at_ms: u64) {
        self.send_actions(vec![AppAction::ScheduledSendDispatchFailed {
            scheduled_id,
            retry_at_ms,
        }])
        .await;
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
            Ok(()) => {
                self.send_actions(vec![AppAction::ScheduledSendCancelled { scheduled_id }])
                    .await;
            }
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
        thread_root_event_id: Option<String>,
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
            .send_server_delayed_message(
                session,
                &room_id,
                thread_root_event_id.as_deref(),
                &body,
                send_at_ms,
            )
            .await
        {
            Ok(delay_id) => {
                self.send_actions(vec![AppAction::ScheduledSendRescheduled {
                    scheduled_id,
                    send_at_ms,
                    handle: ScheduledSendHandle::Server { delay_id },
                }])
                .await;
            }
            Err(()) => {
                self.send_actions(vec![
                    AppAction::ScheduledSendCapabilityChanged {
                        capability: ScheduledSendCapability::LocalFallback,
                    },
                    AppAction::ScheduledSendRescheduled {
                        scheduled_id,
                        send_at_ms,
                        handle: ScheduledSendHandle::Local,
                    },
                ])
                .await;
            }
        }
    }

    async fn send_server_delayed_message(
        &self,
        session: &MatrixClientSession,
        room_id: &str,
        thread_root_event_id: Option<&str>,
        body: &str,
        send_at_ms: u64,
    ) -> Result<String, ()> {
        use matrix_sdk::ruma::TransactionId;
        use matrix_sdk::ruma::api::client::delayed_events::{
            DelayParameters, delayed_message_event,
        };

        let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| ())?;
        let content =
            build_scheduled_message_content(body, thread_root_event_id).map_err(|_| ())?;
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

    async fn emit_search_failed(
        &self,
        request_id: RequestId,
        query: &str,
        scope: &crate::command::SearchScope,
        message: &str,
    ) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
                query: query.to_owned(),
                scope: state_search_scope(scope),
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

    fn record_event_cache_repair(
        request_id: RequestId,
        stage: &'static str,
        outcome: &'static str,
        reason: &'static str,
    ) {
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.event_cache_repair", stage)
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token("outcome", outcome))
                .field(DiagnosticField::token("reason", reason)),
        );
    }

    async fn handle_ensure_room_event_cached(
        &mut self,
        request_id: RequestId,
        room_id: String,
        event_id: String,
    ) {
        let Some(session) = &self.session else {
            Self::record_event_cache_repair(request_id, "skip", "skipped", "no_session");
            return;
        };
        let Ok(parsed_room_id) = matrix_sdk::ruma::RoomId::parse(room_id.as_str()) else {
            Self::record_event_cache_repair(request_id, "skip", "skipped", "invalid_room");
            return;
        };
        let Ok(parsed_event_id) = matrix_sdk::ruma::EventId::parse(event_id.as_str()) else {
            Self::record_event_cache_repair(request_id, "skip", "skipped", "invalid_event");
            return;
        };
        let Some(room) = session.client().get_room(&parsed_room_id) else {
            Self::record_event_cache_repair(request_id, "skip", "skipped", "room_missing");
            return;
        };

        match room.load_or_fetch_event(&parsed_event_id, None).await {
            Ok(_) => {
                Self::record_event_cache_repair(request_id, "done", "succeeded", "loaded");
                if let Some(account_key) = self.active_account_key() {
                    self.route_timeline_command(TimelineCommand::RepairGaps {
                        request_id,
                        key: TimelineKey {
                            account_key,
                            kind: TimelineKind::Room { room_id },
                        },
                    })
                    .await;
                }
            }
            Err(_) => {
                Self::record_event_cache_repair(request_id, "failed", "failed", "sdk");
            }
        }
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
        // #161: jump-to-date renders the focused timeline in the MAIN pane
        // (marked by `main_timeline_anchor`), reusing the focused-context
        // subscription lifecycle; it must not open the right panel.
        let _ = self
            .action_tx
            .send(vec![
                AppAction::OpenFocusedContext {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                },
                AppAction::EnterAnchoredTimeline {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                },
            ])
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
        self.stop_recovery_task().await;
        self.stop_provisional_runtime().await;
        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        self.stop_session_change_observer().await;
        self.record_lifecycle_probe("shutdown_stop_timeline_actor");
        self.stop_timeline_actor().await;
        self.stop_threads_list_actor().await;
        self.record_lifecycle_probe("shutdown_stop_search_actor");
        self.stop_search_actor().await;
        self.record_lifecycle_probe("shutdown_stop_sync_actor");
        self.stop_sync_actor().await;
        self.record_lifecycle_probe("shutdown_clear_room_session");
        self.clear_room_actor_session().await;
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;
        self.invalidate_account_hydration();
        self.abort_avatar_fetch_tasks();
        self.device_session_ordinals.clear();
        self.pending_uia_operations.clear();
        self.provisional_persistable = None;
        self.session_promoted = false;
        self.pending_ready_events.clear();
        self.pending_trust_transition = None;
    }

    /// Ordered shutdown of the ThreadsListActor. Dropping the handle cancels
    /// the actor and its SDK subscriptions.
    async fn stop_threads_list_actor(&mut self) {
        let _ = self.threads_list_actor.take();
    }

    /// Ordered shutdown of the TimelineManagerActor (step 2 of the shutdown
    /// sequence per Async rule 12 — timelines before search/room/sync).
    async fn stop_timeline_actor(&mut self) {
        let (acknowledged, acknowledgement) = tokio::sync::oneshot::channel();
        if self
            .timeline_manager
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
        {
            let _ = acknowledgement.await;
        }
    }

    /// Route a SyncCommand to the SyncActor, or emit SessionRequired if no
    /// store-backed session is active yet.
    async fn route_sync_command(&mut self, command: SyncCommand) {
        let (command_kind, request_id) = match &command {
            SyncCommand::Start { request_id } => ("start", *request_id),
            SyncCommand::Stop { request_id } => ("stop", *request_id),
            SyncCommand::Restart { request_id } => ("restart", *request_id),
            SyncCommand::SyncOnce { request_id } => ("sync_once", *request_id),
        };
        trace_restore!(
            "route_sync_command",
            [
                DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence
                ),
                DiagnosticField::token("kind", command_kind),
                DiagnosticField::boolean("session", self.session.is_some()),
                DiagnosticField::boolean("sync_actor", self.sync_actor.is_some()),
                DiagnosticField::token("action", "begin"),
            ],
            "request_id={} kind={} session={} sync_actor={} action=begin",
            request_id_trace_label(request_id),
            command_kind,
            if self.session.is_some() { "yes" } else { "no" },
            if self.sync_actor.is_some() {
                "yes"
            } else {
                "no"
            }
        );

        if restricted_sync_blocks_sync_once(self.restricted_sync.is_some(), &command) {
            self.emit_failure(
                request_id,
                CoreFailure::SyncFailed {
                    kind: SyncFailureKind::Internal,
                },
            );
            return;
        }

        if self.sync_actor.is_none()
            && !matches!(command, SyncCommand::Stop { .. })
            && let Some(session) = &self.session
        {
            trace_restore!(
                "route_sync_command",
                [
                    DiagnosticField::request_id(
                        "request_id",
                        request_id.connection_id.0,
                        request_id.sequence
                    ),
                    DiagnosticField::token("kind", command_kind),
                    DiagnosticField::token("action", "spawn_sync_actor"),
                ],
                "request_id={} kind={} action=spawn_sync_actor",
                request_id_trace_label(request_id),
                command_kind
            );
            self.spawn_sync_actor(session.clone()).await;
        }

        match &self.sync_actor {
            Some(handle) => {
                trace_restore!(
                    "route_sync_command",
                    [
                        DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence
                        ),
                        DiagnosticField::token("kind", command_kind),
                        DiagnosticField::token("action", "send_to_sync_actor"),
                    ],
                    "request_id={} kind={} action=send_to_sync_actor",
                    request_id_trace_label(request_id),
                    command_kind
                );
                // The SyncActor notifies the RoomActor itself on start/stop/
                // restart: only it knows the selected backend and owns the
                // live RoomListService (canon, overview.md RoomActor bullet).
                let _ = handle.send(SyncMessage::Command(command)).await;
            }
            None if self.session.is_none() => {
                trace_restore!(
                    "route_sync_command",
                    [
                        DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence
                        ),
                        DiagnosticField::token("kind", command_kind),
                        DiagnosticField::token("action", "session_required"),
                    ],
                    "request_id={} kind={} action=session_required",
                    request_id_trace_label(request_id),
                    command_kind
                );
                // Session not yet ready — gate is enforced in AppActor but be
                // defensive here too.
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
            None => {
                trace_restore!(
                    "route_sync_command",
                    [
                        DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence
                        ),
                        DiagnosticField::token("kind", command_kind),
                        DiagnosticField::token("action", "no_sync_actor"),
                    ],
                    "request_id={} kind={} action=no_sync_actor",
                    request_id_trace_label(request_id),
                    command_kind
                );
            }
        }
    }

    /// Spawn the SyncActor for the just-established store-backed session and
    /// notify the RoomActor so room operations become available.
    /// Also replace the TimelineManagerActor with one that holds the session.
    /// Also spawn the SearchActor (Phase 6).
    async fn spawn_sync_actor(&mut self, session: Arc<MatrixClientSession>) {
        trace_restore_simple("spawn_sync_actor", "begin");
        // Give the RoomActor the session so room ops work even before sync
        // starts. The room-list observation starts later, on the SyncActor's
        // RoomMessage::SyncStarted (which carries the live RoomListService on
        // the SyncService backend).
        let _ = self
            .room_actor
            .send(RoomMessage::SessionEstablished {
                session: session.clone(),
            })
            .await;

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
        self.timeline_manager
            .try_send(TimelineMessage::Shutdown { acknowledged: None });
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
            self.sync_generation.clone(),
        );
        self.sync_actor = Some(handle);
        trace_restore_simple("spawn_sync_actor", "done");
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

    async fn start_incoming_verification_observer(&mut self, session: Arc<MatrixClientSession>) {
        self.incoming_verification_session_generation = self
            .incoming_verification_session_generation
            .wrapping_add(1);
        let generation = self.incoming_verification_session_generation;
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut observer = koushi_sdk::observe_incoming_verification_requests(&session).await;
        let mut receiver = observer
            .take_receiver()
            .expect("incoming verification observer receiver is available once");
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    request = receiver.recv() => {
                        let Some(request) = request else { break };
                        let (target, handle) = request.into_parts();
                        if !send_incoming_verification_message_until_stopped(
                            &tx,
                            AccountMessage::IncomingVerificationRequest {
                                generation,
                                target,
                                handle,
                            },
                            &mut stop_rx,
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
        });
        self.incoming_verification_observer = Some(IncomingVerificationObservation {
            stop_tx,
            task,
            observer,
        });
    }

    fn start_session_change_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut changes = session.client().subscribe_to_session_changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    change = changes.recv() => {
                        match change {
                            Ok(matrix_sdk::SessionChange::UnknownToken(data)) => {
                                if tx
                                    .send(AccountMessage::SessionInvalidated {
                                        soft_logout: data.soft_logout,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                break;
                            }
                            Ok(matrix_sdk::SessionChange::TokensRefreshed) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });
        self.session_change_observer = Some(SessionChangeObservation { stop_tx, task });
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
        self.incoming_verification_session_generation = self
            .incoming_verification_session_generation
            .wrapping_add(1);
        if let Some(observation) = self.incoming_verification_observer.take() {
            stop_incoming_verification_observation(observation).await;
        }
    }

    async fn stop_session_change_observer(&mut self) {
        if let Some(observation) = self.session_change_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn handle_session_invalidated(&mut self, soft_logout: bool) {
        trace_restore!(
            "session_invalidated",
            [
                DiagnosticField::boolean("soft_logout", soft_logout),
                DiagnosticField::token("action", "lock"),
            ],
            "soft_logout={} action=lock",
            bool_trace_label(soft_logout)
        );
        if self.session.is_none() {
            return;
        }

        self.send_actions(vec![AppAction::SessionLocked]).await;
        self.invalidate_account_hydration();
        self.stop_sync_actor().await;
    }

    async fn cancel_identity_reset_handle(&mut self) {
        self.identity_reset_flow_id = None;
        if let Some(task) = self.identity_reset_timeout_task.take() {
            task.abort();
        }
        if let Some(handle) = self.identity_reset_handle.take() {
            handle.cancel().await;
        }
    }

    fn spawn_identity_reset_auth_timeout(&mut self, flow_id: u64) {
        if let Some(task) = self.identity_reset_timeout_task.take() {
            task.abort();
        }
        let tx = self.self_tx.clone();
        self.identity_reset_timeout_task = Some(executor::spawn(async move {
            executor::sleep(IDENTITY_RESET_AUTH_TIMEOUT).await;
            let _ = tx
                .send(AccountMessage::IdentityResetAuthTimedOut { flow_id })
                .await;
        }));
    }

    fn clear_identity_reset_handle_after_completion(&mut self) {
        self.identity_reset_flow_id = None;
        if let Some(task) = self.identity_reset_timeout_task.take() {
            task.abort();
        }
        self.identity_reset_handle = None;
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
        self.stop_sas_timeout().await;
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        if let Some(pending) = self.sas_verification.take() {
            let _ = koushi_sdk::cancel_sas_verification(&pending.handle).await;
        }
        if let Some(pending) = self.verification_request.take() {
            let _ = koushi_sdk::cancel_verification_request(&pending.handle).await;
        }
        if let Some((_, handle)) = self.own_user_verification.take() {
            let _ = koushi_sdk::cancel_own_user_sas_verification(&handle).await;
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
                callback_url,
            } => {
                self.handle_complete_oidc_login(request_id, callback_url)
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
                self.handle_query_saved_sessions(request_id).await;
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
                self.handle_probe_local_encryption_health(request_id).await;
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
            AccountCommand::StartSessionBootstrap {
                request_id,
                flow_id,
                auth,
                request,
            } => {
                self.handle_start_session_bootstrap(request_id, flow_id, auth, request)
                    .await;
            }
            AccountCommand::ConfirmSessionBootstrapSaved {
                request_id: _,
                flow_id: _,
            } => {
                self.request_authoritative_trust_recheck().await;
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
            #[cfg(feature = "qa-bin")]
            AccountCommand::QaRefreshDeviceKeysAndAssertKnown {
                target,
                acknowledged,
                ..
            } => {
                let result = match self.session.as_ref() {
                    Some(session) => refresh_device_keys_and_assert_known(session, target).await,
                    None => Err(()),
                };
                let _ = acknowledged.send(result);
            }
            #[cfg(feature = "qa-bin")]
            AccountCommand::QaSetLocalDeviceBlacklisted {
                target,
                room_id,
                acknowledged,
                ..
            } => {
                let result = async {
                    let session = self.session.as_ref().ok_or(())?;
                    let user_id =
                        matrix_sdk::ruma::UserId::parse(target.user_id).map_err(|_| ())?;
                    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(target.device_id);
                    let device = session
                        .client()
                        .encryption()
                        .get_device(&user_id, &device_id)
                        .await
                        .map_err(|_| ())?
                        .ok_or(())?;
                    device
                        .set_local_trust(matrix_sdk_base::crypto::LocalTrust::BlackListed)
                        .await
                        .map_err(|_| ())?;
                    let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| ())?;
                    let room = session.client().get_room(&room_id).ok_or(())?;
                    room.discard_room_key().await.map_err(|_| ())
                }
                .await;
                let _ = acknowledged.send(result);
            }
            AccountCommand::ResetIdentity { request_id } => {
                self.handle_reset_identity(request_id).await;
            }
            AccountCommand::CancelIdentityReset {
                request_id,
                flow_id,
            } => {
                self.handle_cancel_identity_reset(request_id, flow_id).await;
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
            AccountCommand::StartOwnUserSas {
                request_id,
                flow_id,
            } => {
                self.handle_start_own_user_sas(request_id, flow_id).await;
            }
            AccountCommand::RetryCurrentDeviceTrustDiscovery { request_id: _ } => {
                let current_trust = self
                    .session
                    .as_ref()
                    .map(|session| session.current_device_trust());
                let discovery_task_active = self.verification_method_discovery_task.is_some();
                if retry_should_restart_method_discovery(
                    self.session_promoted,
                    current_trust,
                    discovery_task_active,
                    self.verification_method_discovery_failed,
                ) {
                    if self.verification_method_discovery_failed {
                        self.send_actions(vec![
                            AppAction::VerificationMethodDiscoveryRetryStarted {
                                generation: self.trust_generation,
                            },
                        ])
                        .await;
                    }
                    self.discover_verification_methods(self.trust_generation)
                        .await;
                } else {
                    self.request_authoritative_trust_recheck().await;
                }
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
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
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
                self.send_actions(vec![AppAction::VerificationRequested {
                    request_id: request_id.sequence,
                    target: target.clone(),
                }])
                .await;
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

    async fn handle_start_own_user_sas(&mut self, request_id: RequestId, flow_id: u64) {
        let Some(session) = self.session.clone() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        self.cancel_verification_handles().await;
        let own_handle =
            match koushi_sdk::request_own_user_sas_verification(&session, flow_id).await {
                Ok(handle) => handle,
                Err(error) => {
                    self.send_actions(vec![AppAction::VerificationFailed {
                        request_id: flow_id,
                        kind: classify_e2ee_trust_error(&error),
                    }])
                    .await;
                    return;
                }
            };
        let sas = match run_own_user_sas_start(
            flow_id,
            "initial",
            koushi_sdk::start_own_user_sas_verification(&own_handle),
        )
        .await
        {
            Ok(Some(sas)) => sas,
            Ok(None) => {
                self.own_user_verification = Some((flow_id, own_handle));
                self.start_sas_timeout(flow_id);
                self.observe_own_user_verification(request_id, flow_id);
                return;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }])
                .await;
                return;
            }
        };
        self.own_user_verification = Some((flow_id, own_handle));
        self.store_sas_verification(
            RequestId {
                connection_id: request_id.connection_id,
                sequence: flow_id,
            },
            VerificationTarget {
                user_id: "current-user".to_owned(),
                device_id: "eligible-device".to_owned(),
            },
            sas,
        )
        .await;
    }

    async fn recheck_own_user_sas_after_sync(&mut self) {
        if self.sas_verification.is_some() {
            return;
        }
        let Some((flow_id, handle)) = self.own_user_verification.as_ref() else {
            return;
        };
        let state = handle.state();
        if !matches!(state, koushi_sdk::MatrixVerificationRequestState::Ready) {
            return;
        }
        let flow_id = *flow_id;
        let handle = handle.clone();
        match run_own_user_sas_start(
            flow_id,
            "restricted_sync",
            koushi_sdk::start_own_user_sas_verification(&handle),
        )
        .await
        {
            Ok(Some(sas)) => {
                self.store_sas_verification(
                    RequestId {
                        connection_id: RuntimeConnectionId(0),
                        sequence: flow_id,
                    },
                    VerificationTarget {
                        user_id: "current-user".to_owned(),
                        device_id: "eligible-device".to_owned(),
                    },
                    sas,
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }])
                .await;
            }
        }
    }

    fn start_sas_timeout(&mut self, flow_id: u64) {
        if let Some(task) = self.sas_timeout_task.take() {
            task.abort();
        }
        let tx = self.self_tx.clone();
        self.sas_timeout_task = Some(executor::spawn(async move {
            executor::sleep(Duration::from_secs(120)).await;
            let _ = tx
                .send(AccountMessage::SasVerificationTimedOut { flow_id })
                .await;
        }));
    }

    async fn stop_sas_timeout(&mut self) {
        if let Some(task) = self.sas_timeout_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    async fn handle_sas_verification_timeout(&mut self, flow_id: u64) {
        let active = self
            .sas_verification
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == flow_id)
            || self
                .own_user_verification
                .as_ref()
                .is_some_and(|(active_flow_id, _)| *active_flow_id == flow_id);
        if !active {
            return;
        }
        record_sas_verification_event(sas_verification_event("timeout_fired", flow_id));
        self.settle_verification(
            flow_id,
            VerificationTerminal::Failed(TrustOperationFailureKind::Timeout),
        )
        .await;
    }

    fn observe_own_user_verification(&mut self, request_id: RequestId, flow_id: u64) {
        let Some((_, handle)) = self.own_user_verification.as_ref() else {
            return;
        };
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let request_id = RequestId {
            connection_id: request_id.connection_id,
            sequence: flow_id,
        };
        let target = VerificationTarget {
            user_id: "current-user".to_owned(),
            device_id: "eligible-device".to_owned(),
        };
        let task = executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else {
                            let _ = tx.send(AccountMessage::VerificationRequestObserverEnded {
                                flow_id: request_id.sequence,
                            }).await;
                            break;
                        };
                        let terminal = matches!(state,
                            koushi_sdk::MatrixVerificationRequestState::Done
                            | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
                            | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod);
                        if tx.send(AccountMessage::VerificationRequestProgress {
                            request_id,
                            target: target.clone(),
                            state,
                        }).await.is_err() { break; }
                        if terminal { break; }
                    }
                }
            }
        });
        self.verification_request_observer = Some(VerificationObservation { stop_tx, task });
    }

    async fn handle_incoming_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    ) {
        let active_request = self
            .verification_request
            .as_ref()
            .map(|pending| (&pending.target, pending.handle.flow_id()));
        let decision = classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request,
                sas_active: self.sas_verification.is_some(),
                own_user_active: self.own_user_verification.is_some(),
            },
            &target,
            handle.flow_id(),
        );
        match decision {
            IncomingVerificationRequestDecision::Adopt => {}
            IncomingVerificationRequestDecision::Replay => return,
            IncomingVerificationRequestDecision::Conflict => {
                let _ = koushi_sdk::cancel_verification_request(&handle).await;
                return;
            }
        }

        self.verification_request = Some(PendingVerificationRequest {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.observe_verification_request(request_id, target.clone(), handle.clone());
        self.send_actions(vec![AppAction::VerificationRequested {
            request_id: request_id.sequence,
            target: target.clone(),
        }])
        .await;
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
                if let Some(action @ AppAction::LocalUserAliasesLoaded { .. }) =
                    local_user_aliases_action_from_session(session).await
                {
                    self.send_actions(vec![
                        AppAction::LocalUserAliasUpdateFailed {
                            request_id: request_id.sequence,
                            message: "local alias update failed".to_owned(),
                        },
                        action,
                    ])
                    .await;
                } else {
                    self.send_actions(vec![AppAction::LocalUserAliasUpdateFailed {
                        request_id: request_id.sequence,
                        message: "local alias update failed".to_owned(),
                    }])
                    .await;
                }
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

    fn spawn_account_hydration(&mut self, session: Arc<MatrixClientSession>) {
        self.invalidate_account_hydration();
        let generation = self.account_hydration_generation;
        let self_tx = self.self_tx.clone();
        self.account_hydration_task = Some(crate::executor::spawn(async move {
            let (actions, ignored_user_ids) =
                account_hydration_actions_from_session(&session).await;
            if actions.is_empty() {
                return;
            }
            let _ = self_tx
                .send(AccountMessage::AccountHydrationLoaded {
                    generation,
                    actions,
                    ignored_user_ids,
                })
                .await;
        }));
    }

    fn invalidate_account_hydration(&mut self) {
        self.account_hydration_generation = self.account_hydration_generation.wrapping_add(1);
        if let Some(task) = self.account_hydration_task.take() {
            task.abort();
        }
    }

    async fn handle_account_hydration_loaded(
        &mut self,
        generation: u64,
        actions: Vec<AppAction>,
        ignored_user_ids: Option<BTreeSet<String>>,
    ) {
        if generation != self.account_hydration_generation || self.session.is_none() {
            return;
        }
        self.account_hydration_task = None;
        if let Some(user_ids) = ignored_user_ids {
            let _ = self
                .timeline_manager
                .send(TimelineMessage::IgnoredUsersUpdated { user_ids })
                .await;
        }
        self.send_actions(actions).await;
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
            | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
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
        if self
            .recovery_task
            .as_ref()
            .is_some_and(|pending| pending.flow_id == flow_id)
        {
            self.stop_recovery_task().await;
            self.send_actions(vec![AppAction::VerificationGateAttemptFailed {
                kind: koushi_state::VerificationGateFailureKind::Cancelled,
            }])
            .await;
            return;
        }
        if self.active_verification_target(flow_id).is_some() {
            self.settle_verification(flow_id, VerificationTerminal::Cancelled(reason))
                .await;
            return;
        }
        enum CancelTarget {
            Sas {
                target: VerificationTarget,
                handle: koushi_sdk::MatrixSasVerificationHandle,
            },
            Request {
                target: VerificationTarget,
                handle: koushi_sdk::MatrixVerificationRequestHandle,
            },
            Own {
                handle: koushi_sdk::MatrixOwnUserVerificationHandle,
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
            VerificationCancelReason::User => sas_target
                .or_else(|| {
                    self.verification_request
                        .as_ref()
                        .filter(|pending| pending.request_id.sequence == flow_id)
                        .map(|pending| CancelTarget::Request {
                            target: pending.target.clone(),
                            handle: pending.handle.clone(),
                        })
                })
                .or_else(|| {
                    self.own_user_verification
                        .as_ref()
                        .filter(|(active_flow_id, _)| *active_flow_id == flow_id)
                        .map(|(_, handle)| CancelTarget::Own {
                            handle: handle.clone(),
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
            CancelTarget::Own { .. } => VerificationTarget {
                user_id: "current-user".to_owned(),
                device_id: "eligible-device".to_owned(),
            },
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
            CancelTarget::Own { handle } => {
                koushi_sdk::cancel_own_user_sas_verification(&handle).await
            }
        };

        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;
        self.own_user_verification = None;

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
                        let Some(state) = state else {
                            let _ = tx.send(AccountMessage::VerificationRequestObserverEnded {
                                flow_id: request_id.sequence,
                            }).await;
                            break;
                        };
                        let terminal = matches!(
                            state,
                            koushi_sdk::MatrixVerificationRequestState::Done
                                | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
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
                        let Some(state) = state else {
                            let _ = tx.send(AccountMessage::SasVerificationObserverEnded {
                                flow_id: request_id.sequence,
                            }).await;
                            break;
                        };
                        let terminal = matches!(
                            state,
                            koushi_sdk::MatrixSasState::Done
                                | koushi_sdk::MatrixSasState::Cancelled { .. }
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
            && !self
                .own_user_verification
                .as_ref()
                .is_some_and(|(flow_id, _)| *flow_id == request_id.sequence)
        {
            return;
        }
        let mut event = sas_verification_event("request_state_changed", request_id.sequence).field(
            DiagnosticField::token("state", verification_request_state_token(&state)),
        );
        if let koushi_sdk::MatrixVerificationRequestState::Cancelled {
            kind,
            cancelled_by_us,
        } = &state
        {
            event = event
                .field(DiagnosticField::token(
                    "cancel_kind",
                    verification_cancel_kind_token(*kind),
                ))
                .field(DiagnosticField::boolean(
                    "cancelled_by_us",
                    *cancelled_by_us,
                ));
        }
        record_sas_verification_event(event);
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
        record_sas_verification_event(sas_state_changed_event(request_id.sequence, &state));
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
                self.send_actions(vec![AppAction::VerificationAccepted {
                    request_id: request_id.sequence,
                }])
                .await;
                if let Some((flow_id, handle)) = self.own_user_verification.as_ref()
                    && *flow_id == request_id.sequence
                    && self.sas_verification.is_none()
                {
                    let handle = handle.clone();
                    match run_own_user_sas_start(
                        request_id.sequence,
                        "request_ready",
                        koushi_sdk::start_own_user_sas_verification(&handle),
                    )
                    .await
                    {
                        Ok(Some(sas)) => {
                            self.store_sas_verification(
                                request_id,
                                VerificationTarget {
                                    user_id: "current-user".to_owned(),
                                    device_id: "eligible-device".to_owned(),
                                },
                                sas,
                            )
                            .await;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            self.send_actions(vec![AppAction::VerificationFailed {
                                request_id: request_id.sequence,
                                kind: classify_e2ee_trust_error(&error),
                            }])
                            .await;
                        }
                    }
                }
            }
            koushi_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                let target = self
                    .verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == request_id.sequence)
                    .map(|pending| pending.target.clone())
                    .or_else(|| {
                        self.own_user_verification
                            .as_ref()
                            .and_then(|(flow_id, _)| {
                                (*flow_id == request_id.sequence).then(|| VerificationTarget {
                                    user_id: "current-user".to_owned(),
                                    device_id: "eligible-device".to_owned(),
                                })
                            })
                    });
                let Some(target) = target else {
                    return;
                };
                self.store_sas_verification(request_id, target, sas).await;
            }
            koushi_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(request_id).await;
            }
            koushi_sdk::MatrixVerificationRequestState::Cancelled {
                kind,
                cancelled_by_us,
            } => {
                let _ = (kind, cancelled_by_us);
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
        let active_flow_id = self
            .sas_verification
            .as_ref()
            .map(|pending| pending.request_id.sequence);
        let (decision, conflict_rejection_succeeded) =
            resolve_sas_adoption(active_flow_id, request_id.sequence, || async {
                koushi_sdk::cancel_sas_verification(&handle).await.is_ok()
            })
            .await;
        match decision {
            SasAdoptionDecision::Adopt => {}
            SasAdoptionDecision::Replay => return,
            SasAdoptionDecision::Conflict => {
                record_sas_verification_event(
                    sas_verification_event("conflicting_sas_rejected", request_id.sequence).field(
                        DiagnosticField::token(
                            "outcome",
                            if conflict_rejection_succeeded == Some(true) {
                                "success"
                            } else {
                                "failed"
                            },
                        ),
                    ),
                );
                return;
            }
        }

        self.stop_sas_verification_observer().await;
        self.sas_verification = Some(PendingSasVerification {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.start_sas_timeout(request_id.sequence);
        self.observe_sas_verification(request_id, target.clone(), handle.clone());
        let initial_state = handle.state();
        record_sas_verification_event(sas_state_changed_event(request_id.sequence, &initial_state));
        if matches!(initial_state, koushi_sdk::MatrixSasState::Started)
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
                if emojis.len() != 7 {
                    self.project_verification_failure(
                        request_id.sequence,
                        target,
                        TrustOperationFailureKind::Sdk,
                    )
                    .await;
                    return;
                }
                let own_user_flow = self
                    .own_user_verification
                    .as_ref()
                    .is_some_and(|(flow_id, _)| *flow_id == request_id.sequence);
                let action =
                    sas_projection_action(own_user_flow, request_id.sequence, emojis.clone());
                self.send_actions(vec![action]).await;
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
            koushi_sdk::MatrixSasState::Cancelled { .. } => {
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

    fn active_verification_target(&self, flow_id: u64) -> Option<VerificationTarget> {
        self.sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
            .map(|pending| pending.target.clone())
            .or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == flow_id)
                    .map(|pending| pending.target.clone())
            })
            .or_else(|| {
                self.own_user_verification
                    .as_ref()
                    .and_then(|(active_flow_id, _)| {
                        (*active_flow_id == flow_id).then(|| VerificationTarget {
                            user_id: "current-user".to_owned(),
                            device_id: "eligible-device".to_owned(),
                        })
                    })
            })
            .or_else(|| {
                #[cfg(test)]
                {
                    return self.synthetic_verification.as_ref().and_then(
                        |(active_flow_id, target)| {
                            (*active_flow_id == flow_id).then(|| target.clone())
                        },
                    );
                }
                #[cfg(not(test))]
                None
            })
    }

    async fn settle_verification(&mut self, flow_id: u64, terminal: VerificationTerminal) {
        let Some(target) = self.active_verification_target(flow_id) else {
            return;
        };
        let mut event = sas_verification_event("settled", flow_id).field(DiagnosticField::token(
            "terminal",
            verification_terminal_token(terminal),
        ));
        match terminal {
            VerificationTerminal::Success => {}
            VerificationTerminal::Cancelled(reason) => {
                event = event.field(DiagnosticField::token(
                    "reason",
                    verification_cancel_reason_token(reason),
                ));
            }
            VerificationTerminal::Failed(kind) => {
                event = event.field(DiagnosticField::token(
                    "failure_kind",
                    trust_failure_token(kind),
                ));
            }
        }
        record_sas_verification_event(event);
        self.stop_sas_timeout().await;
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        let sas = self.sas_verification.take();
        let request = self.verification_request.take();
        let own = self.own_user_verification.take();
        #[cfg(test)]
        {
            self.synthetic_verification = None;
        }

        if !matches!(terminal, VerificationTerminal::Success) {
            if let Some(pending) = sas.as_ref() {
                let _ = match terminal {
                    VerificationTerminal::Cancelled(VerificationCancelReason::Mismatch) => {
                        koushi_sdk::mismatch_sas_verification(&pending.handle).await
                    }
                    _ => koushi_sdk::cancel_sas_verification(&pending.handle).await,
                };
            } else if let Some(pending) = request.as_ref() {
                let _ = koushi_sdk::cancel_verification_request(&pending.handle).await;
            } else if let Some((_, handle)) = own.as_ref() {
                let _ = koushi_sdk::cancel_own_user_sas_verification(handle).await;
            }
        }
        drop(sas);
        drop(request);
        drop(own);

        match terminal {
            VerificationTerminal::Success => {
                self.send_actions(vec![AppAction::VerificationCompleted {
                    request_id: flow_id,
                }])
                .await;
                self.request_authoritative_trust_recheck().await;
                self.emit_verification_progress(VerificationFlowState::Done {
                    request_id: flow_id,
                    target,
                });
            }
            VerificationTerminal::Cancelled(reason) => {
                self.send_actions(vec![AppAction::VerificationCancelled {
                    request_id: flow_id,
                    reason,
                }])
                .await;
            }
            VerificationTerminal::Failed(kind) => {
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }])
                .await;
                self.emit_verification_progress(VerificationFlowState::Failed {
                    request_id: flow_id,
                    target,
                    kind,
                });
            }
        }
    }

    async fn project_verification_completed(&mut self, request_id: RequestId) {
        if self
            .active_verification_target(request_id.sequence)
            .is_none()
        {
            return;
        }
        self.settle_verification(request_id.sequence, VerificationTerminal::Success)
            .await;
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
        if self.active_verification_target(flow_id).is_some() {
            self.settle_verification(flow_id, VerificationTerminal::Failed(kind))
                .await;
        } else {
            self.send_actions(vec![AppAction::VerificationFailed {
                request_id: flow_id,
                kind,
            }])
            .await;
            let failure = if self.session.is_some() {
                CoreFailure::LocalEncryptionUnavailable
            } else {
                CoreFailure::SessionRequired
            };
            self.emit_failure(request_id, failure);
        }
    }

    async fn project_verification_failure(
        &mut self,
        flow_id: u64,
        _target: VerificationTarget,
        kind: TrustOperationFailureKind,
    ) {
        self.settle_verification(flow_id, VerificationTerminal::Failed(kind))
            .await;
    }

    fn emit_verification_progress(&self, state: VerificationFlowState) {
        if let Some(account_key) = self.active_account_key() {
            self.emit(CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress {
                account_key,
                state,
            }));
        }
    }

    async fn handle_cancel_identity_reset(&mut self, _request_id: RequestId, flow_id: u64) {
        if self.identity_reset_flow_id != Some(flow_id) {
            return;
        }
        let account_key = self.active_account_key();
        self.cancel_identity_reset_handle().await;
        self.send_actions(vec![AppAction::ResetIdentityCancelled {
            request_id: flow_id,
        }])
        .await;
        if let Some(account_key) = account_key {
            for event in project_identity_reset_failed_event(
                flow_id,
                account_key,
                TrustOperationFailureKind::Cancelled,
            ) {
                self.emit(event);
            }
        }
    }

    async fn handle_identity_reset_auth_timeout(&mut self, flow_id: u64) {
        if self.identity_reset_flow_id != Some(flow_id) {
            return;
        }
        let account_key = self.active_account_key();
        self.cancel_identity_reset_handle().await;
        self.send_actions(vec![AppAction::ResetIdentityTimedOut {
            request_id: flow_id,
        }])
        .await;
        if let Some(account_key) = account_key {
            for event in project_identity_reset_failed_event(
                flow_id,
                account_key,
                TrustOperationFailureKind::Timeout,
            ) {
                self.emit(event);
            }
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
                self.send_actions(vec![AppAction::ResetIdentityFailed {
                    request_id: flow_id,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        if self.identity_reset_flow_id != Some(flow_id) {
            return;
        }
        let result = match self.identity_reset_handle.as_ref() {
            Some(handle) => koushi_sdk::complete_identity_reset(&session, handle, &request).await,
            None => Err(koushi_sdk::E2eeTrustError::Sdk(
                "identity reset auth continuation missing".to_owned(),
            )),
        };

        drop(request);

        match result {
            Ok(()) => {
                self.clear_identity_reset_handle_after_completion();
                let (actions, events) =
                    project_reset_identity_completed(flow_request_id, account_key);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(flow_request_id, account_key, error);
                self.send_actions(actions).await;
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
                self.send_actions(vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::bootstrap_cross_signing(&session, auth.as_ref()).await;
        let (actions, events) =
            project_bootstrap_cross_signing_result(request_id, account_key, result);
        self.send_actions(actions).await;
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
                self.send_actions(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::enable_key_backup(&session, passphrase.as_ref()).await;
        drop(passphrase);
        let (actions, events) = project_enable_key_backup_result(request_id, account_key, result);
        self.send_actions(actions).await;
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
                self.send_actions(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::restore_key_backup(&session, &request, version.as_deref()).await;
        drop(request);

        let (actions, events) = project_restore_key_backup_result(request_id, account_key, result);
        self.send_actions(actions).await;
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_export_room_keys(&self, request_id: RequestId, request: RoomKeyExportRequest) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
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
                self.send_actions(vec![AppAction::RoomKeyExported {
                    request_id: request_id.sequence,
                    exported_sessions: summary.exported_sessions,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
                    },
                );
            }
        }
    }

    async fn handle_import_room_keys(&self, request_id: RequestId, request: RoomKeyImportRequest) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
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
                self.send_actions(vec![AppAction::RoomKeyImported {
                    request_id: request_id.sequence,
                    imported_count: summary.imported_count,
                    total_count: summary.total_count,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
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
                self.send_actions(vec![AppAction::SecureBackupSetupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
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
                self.send_actions(vec![
                    AppAction::SecureBackupRecoveryKeyReady {
                        request_id: request_id.sequence,
                        delivery,
                    },
                    AppAction::SecureBackupSetupEnabled {
                        request_id: request_id.sequence,
                    },
                ])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::SecureBackupSetupFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
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
                self.send_actions(vec![AppAction::SecureBackupPassphraseChangeFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
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
                self.send_actions(vec![AppAction::SecureBackupPassphraseChanged {
                    request_id: request_id.sequence,
                    delivery,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::SecureBackupPassphraseChangeFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
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
                self.send_actions(vec![AppAction::ResetIdentityFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        match koushi_sdk::reset_identity(&session).await {
            Ok(koushi_sdk::IdentityResetOutcome::Completed) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) = project_reset_identity_completed(request_id, account_key);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
            Ok(koushi_sdk::IdentityResetOutcome::AuthRequired(handle)) => {
                let auth_type = handle.desktop_auth_type();
                self.cancel_identity_reset_handle().await;
                self.identity_reset_flow_id = Some(request_id.sequence);
                self.spawn_identity_reset_auth_timeout(request_id.sequence);
                self.identity_reset_handle = Some(handle);
                let (actions, events) =
                    project_reset_identity_auth_required(request_id, account_key, auth_type);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(request_id, account_key, error);
                self.send_actions(actions).await;
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
                self.send_actions(vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: requested_homeserver,
                    flows: discovery.flows,
                    delegated: discovery.delegated,
                }])
                .await;
            }
            Ok(Err(error)) => {
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver,
                    kind: login_discovery_failure_kind(&error),
                }])
                .await;
            }
            Err(_) => {
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
            }
        }
    }

    async fn handle_start_oidc_login(&mut self, request_id: RequestId, homeserver: String) {
        match koushi_sdk::start_oidc_login(&homeserver, OIDC_REDIRECT_URI).await {
            Ok((pending, authorization)) => {
                self.pending_oidc_login = Some((request_id, PendingOidcFlow::Sdk(pending)));
                self.emit(CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
                    request_id,
                    authorization_url: authorization.authorization_url,
                    state: authorization.state,
                }));
            }
            Err(error) => {
                let kind = classify_auth_error(&error);
                self.send_actions(vec![AppAction::LoginDiscoveryFailed { homeserver, kind }])
                    .await;
                self.emit_failure(request_id, CoreFailure::AccountOperationFailed { kind });
            }
        }
    }

    async fn handle_complete_oidc_login(&mut self, request_id: RequestId, callback_url: String) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let Some((start_request_id, pending)) = self.pending_oidc_login.take() else {
            self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                homeserver: String::new(),
                kind: AuthFailureKind::Cancelled,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Cancelled,
                },
            );
            self.send_actions(vec![AppAction::LoginFailed {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                message: "login failed".to_owned(),
            }])
            .await;
            return;
        };
        let homeserver = pending.homeserver().to_owned();
        self.send_actions(vec![AppAction::AuthenticationStarted {
            attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
            homeserver: homeserver.clone(),
        }])
        .await;

        #[cfg(test)]
        let login_result = match self.oidc_completion_override.take() {
            Some(session) => Ok(session),
            None => match pending {
                PendingOidcFlow::Sdk(pending) => {
                    koushi_sdk::finish_oidc_login(pending, &callback_url).await
                }
                PendingOidcFlow::Synthetic { .. } => {
                    unreachable!("synthetic OIDC completion requires a session override")
                }
            },
        };
        #[cfg(not(test))]
        let login_result = match pending {
            PendingOidcFlow::Sdk(pending) => {
                koushi_sdk::finish_oidc_login(pending, &callback_url).await
            }
        };

        let login_session = match login_result {
            Ok(session) => session,
            Err(error) => {
                let kind = classify_auth_error(&error);
                self.send_actions(vec![AppAction::LoginDiscoveryFailed { homeserver, kind }])
                    .await;
                self.emit_failure(request_id, CoreFailure::AccountOperationFailed { kind });
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        let info = login_session.info.clone();
        let key_id = session_key_id_from_info(&info);
        let account_key = account_key_from_info(&info);

        let persistable = match login_session.persistable_session() {
            Ok(persistable) => persistable,
            Err(_) => {
                self.abort_login(login_session, &key_id, false).await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        let store_backed = match self.restore_into_store(&persistable, &key_id).await {
            Ok(session) => session,
            Err(failure) => {
                self.abort_login(login_session, &key_id, false).await;
                self.emit_failure(request_id, failure);
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        drop(login_session);

        self.install_provisional_session(
            store_backed,
            persistable,
            key_id,
            AppAction::LoginSucceeded {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                info,
            },
        )
        .await;

        self.pending_ready_events
            .push(CoreEvent::Account(AccountEvent::LoggedIn {
                request_id: start_request_id,
                account_key: account_key.clone(),
            }));
        if request_id != start_request_id {
            self.pending_ready_events
                .push(CoreEvent::Account(AccountEvent::LoggedIn {
                    request_id,
                    account_key,
                }));
        }
    }

    async fn handle_login_password(&mut self, request_id: RequestId, request: LoginRequest) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        // Store bootstrap step 1: the password exchange runs on a storeless
        // client. The device id (and therefore the store path) is unknown
        // before this completes. The storeless client must never sync or
        // initialize encryption.
        let login_result = koushi_sdk::login_with_password_with_store(&request, None).await;

        let login_session = match login_result {
            Err(error) => {
                let kind = classify_login_error(&error);
                self.emit_failure(request_id, CoreFailure::LoginFailed { kind });
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
            Ok(session) => session,
        };

        let info = login_session.info.clone();
        let key_id = session_key_id_from_info(&info);
        let account_key = account_key_from_info(&info);

        // Build a restorable in-memory session shape without writing the
        // active credential index or last-session pointer before verification.
        let persistable = match login_session.persistable_session() {
            Ok(persistable) => persistable,
            Err(_) => {
                self.abort_login(login_session, &key_id, false).await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
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
                self.abort_login(login_session, &key_id, false).await;
                self.emit_failure(request_id, failure);
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        // The storeless client never synced; drop it inside the runtime
        // context (Async rule 11).
        drop(login_session);

        self.install_provisional_session(
            store_backed,
            persistable,
            key_id,
            AppAction::LoginSucceeded {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                info,
            },
        )
        .await;

        // Emit domain event carrying the request_id for command correlation.
        self.pending_ready_events
            .push(CoreEvent::Account(AccountEvent::LoggedIn {
                request_id,
                account_key,
            }));
    }

    async fn handle_restore_session(&mut self, request_id: RequestId, account_key: AccountKey) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        trace_account_request("restore_session", request_id, "lookup_key");
        let key_id = match self.lookup_session_key_id(&account_key).await {
            Ok(Some(key_id)) => {
                trace_account_request("restore_session", request_id, "key_found");
                key_id
            }
            Ok(None) => {
                trace_account_request("restore_session", request_id, "key_missing");
                // No stored session for this account: project
                // RestoreSessionNotFound so AppState returns to SignedOut, and
                // keep the redacted failure event for command correlation.
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                trace_account_request("restore_session", request_id, "key_lookup_failed");
                // Credential store unreachable.
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
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
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        trace_account_request("restore_last_session", request_id, "load_pointer");
        let key_id = match self.store.credential_backend().load_last_session() {
            Ok(Some(key_id)) => {
                trace_account_request("restore_last_session", request_id, "pointer_found");
                key_id
            }
            Ok(None) => {
                trace_account_request("restore_last_session", request_id, "pointer_missing");
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                trace_account_request("restore_last_session", request_id, "pointer_load_failed");
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
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
    async fn handle_query_saved_sessions(&self, request_id: RequestId) {
        let store = self.store.clone();
        match executor::spawn_blocking(move || store.credential_backend().load_saved_sessions())
            .await
        {
            Ok(Ok(index)) => {
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
            Ok(Err(_)) | Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            }
        }
    }

    async fn handle_query_devices(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::DeviceSessionsLoadFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
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
                self.send_actions(vec![AppAction::DeviceSessionsLoaded {
                    request_id: request_id.sequence,
                    devices: summaries,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_auth_failure(&error);
                self.send_actions(vec![AppAction::DeviceSessionsLoadFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::AccountOperationFailed { kind });
            }
        }
    }

    async fn handle_load_account_management_capabilities(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::AccountManagementCapabilitiesLoadFailed])
                    .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let capabilities = koushi_sdk::account_management_capabilities(&session).await;
        self.send_actions(vec![AppAction::AccountManagementCapabilitiesLoaded {
            change_password: capabilities.change_password,
        }])
        .await;
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
                )
                .await;
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
            )
            .await;
            return;
        };

        let result = koushi_sdk::rename_device(&session, &raw_device_id, &display_name).await;
        drop(display_name);
        match result {
            Ok(()) => {
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }])
                .await;
            }
            Err(_) => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                )
                .await
            }
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
                )
                .await;
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
                )
                .await;
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
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }])
                .await;
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
                self.send_actions(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }])
                .await;
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
                )
                .await;
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
                )
                .await;
                return;
            }
        };

        let result = koushi_sdk::change_password(&session, &new_password, None, None).await;
        match result {
            Ok(()) => {
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }])
                .await;
            }
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
                self.send_actions(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }])
                .await;
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
                )
                .await;
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
                )
                .await;
                return;
            }
        };

        let result = koushi_sdk::deactivate_account(&session, erase_data, None, None).await;
        match result {
            Ok(()) => {
                self.pending_uia_operations.remove(&request_id.sequence);
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: request_id.sequence,
                    operation,
                }])
                .await;
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
                self.send_actions(vec![AppAction::AccountManagementUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }])
                .await;
            }
            Err(koushi_sdk::AccountManagementError::Sdk(_)) => {
                self.project_account_management_failure(
                    request_id,
                    operation,
                    AuthFailureKind::Sdk,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                )
                .await;
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
                )
                .await;
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
                    )
                    .await;
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
                self.send_actions(vec![AppAction::AccountManagementSucceeded {
                    request_id: flow_id,
                    operation,
                }])
                .await;
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
                )
                .await;
            }
        }
    }

    async fn handle_soft_logout_reauth(
        &mut self,
        request_id: RequestId,
        password: koushi_state::AuthSecret,
    ) {
        let Some(session) = self.session.as_ref() else {
            self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                request_id: request_id.sequence,
                kind: AuthFailureKind::Sdk,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let info = session.info.clone();
        let key_id = session_key_id_from_info(&info);

        // Stop live sync immediately, but keep the locked session until the
        // password login succeeds so a bad password does not make retry impossible.
        self.stop_sync_actor().await;

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
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
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

        let persistable = match self.persist_session(&login_session, &key_id).await {
            Ok(persistable) => persistable,
            Err(failure) => {
                self.abort_login(login_session, &key_id, false).await;
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, failure);
                return;
            }
        };

        // The locked session's observers own SDK streams and therefore keep the old client
        // alive. Stop and join them before replacing the session or subscribing successors.
        self.record_lifecycle_probe("recovery_observer_stop_requested");
        self.stop_recovery_observer().await;
        self.record_lifecycle_probe("recovery_observer_terminated");
        self.record_lifecycle_probe("incoming_verification_observer_stop_requested");
        self.stop_incoming_verification_observer().await;
        self.record_lifecycle_probe("incoming_verification_observer_terminated");
        self.stop_session_change_observer().await;
        self.invalidate_account_hydration();
        drop(self.session.take());
        self.session_key_id = None;

        let store_backed = match self.restore_into_store(&persistable, &key_id).await {
            Ok(session) => session,
            Err(failure) => {
                self.abort_login(login_session, &key_id, true).await;
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
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
        self.record_lifecycle_probe("incoming_verification_observer_subscribing");
        self.start_incoming_verification_observer(session_arc.clone())
            .await;
        self.spawn_sync_actor(session_arc.clone()).await;

        let account_key = account_key_from_info(&info);
        self.send_actions(vec![
            AppAction::SoftLogoutReauthSucceeded {
                request_id: request_id.sequence,
            },
            AppAction::LoginSucceeded {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                info,
            },
        ])
        .await;
        self.emit(CoreEvent::Account(AccountEvent::LoggedIn {
            request_id,
            account_key,
        }));
        self.spawn_account_hydration(session_arc.clone());

        self.start_recovery_observer(session_arc.clone());
        self.record_lifecycle_probe("recovery_observer_started");
        self.start_session_change_observer(session_arc);
    }

    async fn project_account_management_failure(
        &self,
        request_id: RequestId,
        operation: AccountManagementOperation,
        kind: AuthFailureKind,
        failure: CoreFailure,
    ) {
        self.send_actions(vec![AppAction::AccountManagementFailed {
            request_id: request_id.sequence,
            operation,
            kind,
        }])
        .await;
        self.emit_failure(request_id, failure);
    }

    async fn handle_switch_account(&mut self, request_id: RequestId, account_key: AccountKey) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let key_id = match self.lookup_session_key_id(&account_key).await {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                // Same not-found contract as RestoreSession.
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        // Project the switch intent so the reducer drives state
        // (SwitchingAccount → cleared views), then run the store-backed
        // restore of the target account.
        self.send_actions(vec![AppAction::SwitchAccountRequested {
            info: session_info_from_key_id(&key_id),
        }])
        .await;

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
        let restore_started = Some(startup_trace::now());
        trace_account_request("restore_account", request_id, "load_session");
        let session_json = match self.store.credential_backend().load_matrix_session(&key_id) {
            Ok(stored) => stored,
            Err(err) if koushi_key::is_missing_credential_error(&err) => {
                trace_account_request("restore_account", request_id, "session_missing");
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                trace_account_request("restore_account", request_id, "session_load_failed");
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        let persistable = match PersistableMatrixSession::from_json(session_json.as_str()) {
            Ok(s) => s,
            Err(_) => {
                trace_account_request("restore_account", request_id, "session_parse_failed");
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        trace_account_request("restore_account", request_id, "store_restore_begin");
        match self.restore_into_store(&persistable, &key_id).await {
            Err(failure) => {
                trace_account_request("restore_account", request_id, "store_restore_failed");
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, failure);
            }
            Ok(session) => {
                trace_account_request("restore_account", request_id, "store_restore_ok");
                startup_trace::trace_phase(StartupPhase::Restore, restore_started);
                let info = session.info.clone();
                let account_key = account_key_from_info(&info);

                self.install_provisional_session(
                    session,
                    persistable,
                    key_id,
                    AppAction::RestoreSessionSucceeded(info),
                )
                .await;

                self.pending_ready_events
                    .push(CoreEvent::Account(match outcome {
                        RestoreOutcome::Restored => AccountEvent::SessionRestored {
                            request_id,
                            account_key,
                        },
                        RestoreOutcome::Switched => AccountEvent::AccountSwitched {
                            request_id,
                            account_key,
                        },
                    }));
            }
        }
    }

    async fn handle_start_session_bootstrap(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        auth: Option<koushi_state::AuthSecret>,
        request: SecureBackupSetupRequest,
    ) {
        let Some(session) = self.session.clone() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        if request.recovery_key_destination_path.is_none() {
            self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                flow_id,
                kind: koushi_state::VerificationGateFailureKind::Sdk,
            }])
            .await;
            return;
        }
        if let Err(error) = koushi_sdk::bootstrap_cross_signing(&session, auth.as_ref()).await {
            drop(auth);
            self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                flow_id,
                kind: verification_gate_failure_kind(&error),
            }])
            .await;
            return;
        }
        drop(auth);
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
            Ok(summary) if summary.recovery_key_written => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDelivered { flow_id }])
                    .await;
            }
            Ok(_) => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                    flow_id,
                    kind: koushi_state::VerificationGateFailureKind::Sdk,
                }])
                .await;
            }
            Err(error) => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                    flow_id,
                    kind: verification_gate_failure_kind(&error),
                }])
                .await;
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

        self.stop_recovery_task().await;
        let generation = self.trust_generation;
        let flow_id = request_id.sequence;
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            let result = koushi_sdk::recover_e2ee(&session, &request).await;
            drop(request);
            let _ = tx
                .send(AccountMessage::RecoveryFinished {
                    generation,
                    flow_id,
                    request_id,
                    result,
                })
                .await;
        });
        self.recovery_task = Some(PendingRecoveryTask {
            generation,
            flow_id,
            request_id,
            task,
        });
    }

    async fn handle_recovery_finished(
        &mut self,
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
        result: Result<(), koushi_sdk::E2eeRecoveryError>,
    ) {
        let is_current = self.recovery_task.as_ref().is_some_and(|pending| {
            recovery_result_is_current(
                generation,
                self.trust_generation,
                flow_id,
                pending.flow_id,
                request_id,
                pending.request_id,
                self.session.is_some(),
            ) && pending.generation == generation
        });
        if !is_current {
            return;
        }
        if let Some(pending) = self.recovery_task.take() {
            let _ = pending.task.await;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let account_key = AccountKey(session.info.user_id.clone());
        match result {
            Ok(()) => {
                // SDK success remains gated; the subsequent authoritative
                // trust observation is the only promotion authority.
                self.send_actions(vec![AppAction::E2eeRecoverySucceeded])
                    .await;
                self.request_authoritative_trust_recheck().await;
                self.send_actions(vec![AppAction::RestoreKeyBackupRequested {
                    request_id: request_id.sequence,
                    version: None,
                }])
                .await;
                #[cfg(test)]
                let recovery_download_override = self
                    .recovery_download_override
                    .lock()
                    .expect("recovery download lock")
                    .take();
                #[cfg(test)]
                let restore_result = if let Some(completion) = recovery_download_override {
                    if completion.await.unwrap_or(false) {
                        Ok(koushi_sdk::KeyBackupRestoreSummary {
                            scope: koushi_sdk::KeyBackupRestoreScope::JoinedRooms,
                            version: None,
                            restored_rooms: 0,
                            total_rooms: Some(0),
                        })
                    } else {
                        Err(koushi_sdk::E2eeTrustError::Sdk(
                            "controlled recovery download failure".to_owned(),
                        ))
                    }
                } else {
                    koushi_sdk::download_joined_room_keys_from_backup(&session, None).await
                };
                #[cfg(not(test))]
                let restore_result =
                    koushi_sdk::download_joined_room_keys_from_backup(&session, None).await;
                let (actions, events) = project_restore_key_backup_result(
                    request_id,
                    account_key.clone(),
                    restore_result,
                );
                self.send_actions(actions).await;
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
                self.send_actions(vec![AppAction::E2eeRecoveryFailed {
                    message: "recovery failed".to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RecoveryFailed { kind });
            }
        }
    }

    async fn stop_recovery_task(&mut self) -> Option<u64> {
        let pending = self.recovery_task.take()?;
        let flow_id = pending.flow_id;
        pending.task.abort();
        let _ = pending.task.await;
        Some(flow_id)
    }

    async fn request_authoritative_trust_recheck(&self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let _ = self
            .self_tx
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: self.trust_generation,
                trust: session.current_device_trust(),
            })
            .await;
    }

    async fn perform_logout(&mut self, request_id: RequestId, server_logout: bool) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
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

        self.next_teardown_generation = self.next_teardown_generation.wrapping_add(1);
        let generation = self.next_teardown_generation;
        self.pending_session_teardown = Some(PendingSessionTeardown {
            generation,
            attempt: 0,
            session,
            key_id,
            continuation: SessionTeardownContinuation::Logout {
                request_id,
                server_logout,
            },
        });
        self.retry_session_teardown(generation).await;
    }

    async fn close_pending_session_stores(
        &mut self,
        session: &MatrixClientSession,
    ) -> Result<(), ()> {
        #[cfg(test)]
        if let Some(success) = self.close_store_results.pop_front()
            && !success
        {
            return Err(());
        }
        koushi_sdk::close_session_stores(session)
            .await
            .map_err(|_| ())
    }

    async fn retry_session_teardown(&mut self, generation: u64) {
        let Some(pending) = self.pending_session_teardown.as_ref() else {
            return;
        };
        if pending.generation != generation {
            return;
        }
        let session = pending.session.clone();
        if self.close_pending_session_stores(&session).await.is_err() {
            let pending = self
                .pending_session_teardown
                .as_mut()
                .expect("teardown remains pending after close failure");
            pending.attempt = pending.attempt.saturating_add(1);
            let shift = pending.attempt.min(5);
            let delay_ms = 25_u64.saturating_mul(1_u64 << shift).min(1_000);
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.account",
                    "session_store_close_retrying",
                )
                .field(DiagnosticField::count("attempt", pending.attempt as u64)),
            );
            self.record_lifecycle_probe("session_store_close_retrying");
            let tx = self.self_tx.clone();
            self.teardown_retry_task = Some(executor::spawn(async move {
                executor::sleep(Duration::from_millis(delay_ms)).await;
                let _ = tx
                    .send(AccountMessage::RetrySessionTeardown { generation })
                    .await;
            }));
            return;
        }
        if let Some(task) = self.teardown_retry_task.take() {
            task.abort();
        }
        self.record_lifecycle_probe("session_store_closed");
        record(DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.account",
            "session_store_closed",
        ));
        let pending = self
            .pending_session_teardown
            .take()
            .expect("successful teardown remains pending");
        drop(pending.session);
        match pending.continuation {
            SessionTeardownContinuation::Logout {
                request_id,
                server_logout,
            } => {
                let _ = server_logout;
                let account_key = if let Some(key_id) = &pending.key_id {
                    self.clear_account_persistence(key_id).await;
                    AccountKey(key_id.user_id.clone())
                } else {
                    AccountKey(String::new())
                };
                self.record_lifecycle_probe("session_persistence_deleted");
                self.send_actions(vec![AppAction::LogoutFinished]).await;
                self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
                    request_id,
                    account_key,
                }));
            }
            SessionTeardownContinuation::InstallReplacement {
                session,
                persistable,
                key_id,
                action,
            } => {
                // Account replacement must preserve the source account's
                // credentials, saved-session index entry, last pointer, and
                // keyed store. Only its live SDK handles are drained/dropped.
                self.record_lifecycle_probe("replacement_teardown_complete");
                Box::pin(self.install_provisional_session(session, persistable, key_id, action))
                    .await;
            }
        }
    }

    async fn handle_logout(&mut self, request_id: RequestId) {
        self.perform_logout(request_id, true).await;
    }

    // --- helpers ---

    async fn install_provisional_session(
        &mut self,
        session: MatrixClientSession,
        persistable: PersistableMatrixSession,
        key_id: SessionKeyId,
        action: AppAction,
    ) {
        debug_assert!(self.pending_session_teardown.is_none());
        if let Some(previous_session) = self.session.take() {
            let previous_key_id = self.session_key_id.take();
            self.stop_current_session_runtime().await;
            self.next_teardown_generation = self.next_teardown_generation.wrapping_add(1);
            let generation = self.next_teardown_generation;
            self.pending_session_teardown = Some(PendingSessionTeardown {
                generation,
                attempt: 0,
                session: previous_session,
                key_id: previous_key_id,
                continuation: SessionTeardownContinuation::InstallReplacement {
                    session,
                    persistable,
                    key_id,
                    action,
                },
            });
            self.retry_session_teardown(generation).await;
            return;
        }
        self.stop_provisional_runtime().await;
        let session = Arc::new(session);
        self.device_session_ordinals.clear();
        self.pending_uia_operations.clear();
        self.session = Some(session.clone());
        self.session_key_id = Some(key_id);
        self.provisional_persistable = Some(persistable);
        self.session_promoted = false;
        self.send_actions(vec![action]).await;
        self.start_provisional_runtime(session).await;
    }

    async fn start_provisional_runtime(&mut self, session: Arc<MatrixClientSession>) {
        self.trust_generation = self.trust_generation.wrapping_add(1);
        let generation = self.trust_generation;
        let trust_read_started_at = Instant::now();
        #[cfg(any(test, feature = "test-hooks"))]
        let (observation, synthetic_trust_observation) = {
            let override_observation = self
                .trust_observation_override
                .lock()
                .expect("trust observation override lock")
                .take();
            let synthetic = override_observation.is_some();
            (
                override_observation.unwrap_or_else(|| session.observe_current_device_trust()),
                synthetic,
            )
        };
        #[cfg(not(any(test, feature = "test-hooks")))]
        let observation = session.observe_current_device_trust();
        let current_trust = observation.current;
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.trust_observation_is_synthetic = synthetic_trust_observation;
        }
        record_verification_admission_event(
            verification_admission_event("trust_read_finished", generation, 0)
                .field(DiagnosticField::token(
                    "trust",
                    current_device_trust_token(current_trust),
                ))
                .field(DiagnosticField::milliseconds(
                    "elapsed_ms",
                    trust_read_started_at.elapsed().as_millis(),
                )),
        );
        let tx = self.self_tx.clone();
        let mut updates = observation.updates;
        self.trust_observer = Some(executor::spawn(async move {
            while let Some(trust) = updates.next().await {
                if tx
                    .send(AccountMessage::CurrentDeviceTrustChanged { generation, trust })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.handle_current_device_trust(generation, current_trust)
            .await;
    }

    fn start_restricted_sync(
        &mut self,
        session: Arc<MatrixClientSession>,
        generation: u64,
        transition_id: u64,
    ) {
        if !begin_restricted_sync_cursor_attempt(self.restricted_sync.is_some()) {
            return;
        }
        record_verification_admission_event(verification_admission_event(
            "restricted_catch_up_started",
            generation,
            transition_id,
        ));
        #[cfg(any(test, feature = "test-hooks"))]
        if self.trust_observation_is_synthetic {
            let _ = session;
            self.restricted_sync = Some(executor::spawn(std::future::pending()));
            return;
        }
        let tx = self.self_tx.clone();
        self.restricted_sync = Some(executor::spawn(async move {
            let first_sync = executor::timeout(
                Duration::from_secs(15),
                koushi_sdk::restricted_verification_sync_once_with_token(&session, None),
            )
            .await;
            let mut token = match first_sync {
                Ok(Ok(token)) => Some(token),
                _ => None,
            };
            let succeeded = token.is_some();
            if tx
                .send(AccountMessage::FirstRestrictedSyncFinished {
                    generation,
                    succeeded,
                })
                .await
                .is_err()
                || !succeeded
            {
                return;
            }
            let mut failure_reported = false;
            loop {
                match koushi_sdk::restricted_verification_sync_once_with_token(
                    &session,
                    token.clone(),
                )
                .await
                {
                    Ok(next_token) => {
                        should_report_restricted_sync_failure(&mut failure_reported, true);
                        token = Some(next_token);
                        if tx
                            .send(AccountMessage::RestrictedSyncSucceeded { generation })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        if should_report_restricted_sync_failure(&mut failure_reported, false) {
                            if tx
                                .send(AccountMessage::RestrictedSyncFailed { generation })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                executor::sleep(Duration::from_millis(250)).await;
            }
        }));
    }

    async fn stop_provisional_runtime(&mut self) {
        self.trust_generation = self.trust_generation.wrapping_add(1);
        self.cancel_pending_trust_promotion().await;
        if let Some(task) = self.trust_observer.take() {
            task.abort();
            let _ = task.await;
            self.record_lifecycle_probe("trust_observer_terminated");
        }
        if let Some(owned) = self.verification_method_discovery_task.take() {
            owned.task.abort();
            let _ = owned.task.await;
            record_verification_method_discovery_event(verification_method_discovery_event(
                "cancelled",
                owned.generation,
                owned.serial,
            ));
        }
        self.verification_method_discovery_failed = false;
        self.stop_restricted_sync().await;
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.trust_observation_is_synthetic = false;
        }
    }

    async fn stop_restricted_sync(&mut self) {
        if let Some(task) = self.restricted_sync.take() {
            task.abort();
            let _ = task.await;
            self.record_lifecycle_probe("restricted_sync_terminated");
        }
    }

    async fn stop_normal_runtime_children(&mut self) {
        self.record_lifecycle_probe("stop_recovery_observer");
        self.stop_recovery_observer().await;
        self.record_lifecycle_probe("stop_incoming_verification_observer");
        self.stop_incoming_verification_observer().await;
        self.record_lifecycle_probe("stop_session_change_observer");
        self.stop_session_change_observer().await;
        self.record_lifecycle_probe("stop_timeline_manager");
        self.stop_timeline_actor().await;
        self.timeline_manager = crate::timeline::TimelineManagerActor::spawn(
            self.action_tx.clone(),
            self.event_tx.clone(),
            Some(self.data_dir.clone()),
            self.messages_backpressure.clone(),
        );
        self.record_lifecycle_probe("stop_threads_manager");
        self.stop_threads_list_actor().await;
        self.record_lifecycle_probe("stop_search_actor");
        self.stop_search_actor().await;
        self.record_lifecycle_probe("stop_sync_actor");
        self.stop_sync_actor().await;
        self.record_lifecycle_probe("clear_room_session");
        self.clear_room_actor_session().await;
        self.record_lifecycle_probe("abort_hydration");
        self.invalidate_account_hydration();
        self.record_lifecycle_probe("abort_attention_media_tasks");
        self.abort_avatar_fetch_tasks();
    }

    fn record_lifecycle_probe(&self, token: &'static str) {
        #[cfg(test)]
        if let Some(probe) = &self.lifecycle_probe {
            let _ = probe.send(token);
        }
        #[cfg(not(test))]
        let _ = token;
    }

    async fn handle_current_device_trust(
        &mut self,
        generation: u64,
        trust: koushi_state::CurrentDeviceTrustState,
    ) {
        match trust_lifecycle_decision(
            generation,
            self.trust_generation,
            self.session_promoted,
            trust,
        ) {
            TrustLifecycleDecision::IgnoreStale | TrustLifecycleDecision::AlreadyReady => return,
            TrustLifecycleDecision::StayGated => {
                self.cancel_pending_trust_promotion().await;
                let transition_id = self.next_trust_transition_id();
                self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust,
                }])
                .await;
                if self.restricted_sync.is_none()
                    && let Some(session) = self.session.clone()
                {
                    self.start_restricted_sync(session, generation, transition_id);
                }
                return;
            }
            TrustLifecycleDecision::Lock => {
                let transition_id = self.next_trust_transition_id();
                self.pending_trust_transition = Some(PendingTrustTransition {
                    generation,
                    transition_id,
                    decision: TrustLifecycleDecision::Lock,
                });
                self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust,
                }])
                .await;
                return;
            }
            TrustLifecycleDecision::Promote => {}
        }
        if !matches!(trust, koushi_state::CurrentDeviceTrustState::Verified) {
            self.send_actions(vec![AppAction::CurrentDeviceTrustChanged(trust)])
                .await;
            return;
        }
        if matches!(self.pending_trust_transition, Some(PendingTrustTransition { generation: pending_generation, decision: TrustLifecycleDecision::Promote, .. }) if pending_generation == generation)
        {
            return;
        }
        let (Some(session), Some(key_id)) = (self.session.clone(), self.session_key_id.clone())
        else {
            return;
        };
        if self.persist_session(&session, &key_id).await.is_err() {
            self.send_actions(vec![AppAction::SessionPersistenceFailed {
                message: "session persistence failed".to_owned(),
            }])
            .await;
            return;
        }
        self.provisional_persistable = None;
        let transition_id = self.next_trust_transition_id();
        self.pending_trust_transition = Some(PendingTrustTransition {
            generation,
            transition_id,
            decision: TrustLifecycleDecision::Promote,
        });
        let restricted_was_active = self.restricted_sync.is_some();
        self.stop_restricted_sync().await;
        record_verification_admission_event(verification_admission_event(
            if restricted_was_active {
                "restricted_catch_up_stopped"
            } else {
                "restricted_catch_up_skipped"
            },
            generation,
            transition_id,
        ));
        record_verification_admission_event(verification_admission_event(
            "ready_projection_dispatched",
            generation,
            transition_id,
        ));
        self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
            generation,
            transition_id,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        }])
        .await;
    }

    async fn cancel_pending_trust_promotion(&mut self) {
        if matches!(
            self.pending_trust_transition,
            Some(PendingTrustTransition {
                decision: TrustLifecycleDecision::Promote,
                ..
            })
        ) {
            self.pending_trust_transition = None;
        }
    }

    fn next_trust_transition_id(&mut self) -> u64 {
        self.next_trust_transition_id = self.next_trust_transition_id.wrapping_add(1);
        self.next_trust_transition_id
    }

    async fn discover_verification_methods(&mut self, generation: u64) {
        if let Some(owned) = self.verification_method_discovery_task.take() {
            owned.task.abort();
            let _ = owned.task.await;
            record_verification_method_discovery_event(verification_method_discovery_event(
                "cancelled",
                owned.generation,
                owned.serial,
            ));
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.verification_method_discovery_failed = false;
        self.verification_method_discovery_serial =
            self.verification_method_discovery_serial.wrapping_add(1);
        let serial = self.verification_method_discovery_serial;
        let tx = self.self_tx.clone();
        record_verification_method_discovery_event(verification_method_discovery_event(
            "started", generation, serial,
        ));
        let task = executor::spawn(async move {
            let started_at = Instant::now();
            let result = wait_for_verification_method_discovery(
                VERIFICATION_METHOD_DISCOVERY_TIMEOUT,
                koushi_sdk::discover_current_session_verification_methods(&session),
            )
            .await;
            let outcome = match &result {
                VerificationMethodDiscoveryResult::Discovered(_) => "success",
                VerificationMethodDiscoveryResult::Failed(_) => "failed",
            };
            record_verification_method_discovery_event(
                verification_method_discovery_event("finished", generation, serial)
                    .field(DiagnosticField::token("outcome", outcome))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_ms",
                        started_at.elapsed().as_millis(),
                    )),
            );
            let _ = tx
                .send(AccountMessage::VerificationMethodsDiscovered {
                    generation,
                    serial,
                    result,
                })
                .await;
        });
        self.verification_method_discovery_task = Some(OwnedVerificationMethodDiscoveryTask {
            generation,
            serial,
            task,
        });
    }

    async fn handle_trust_projection_applied(
        &mut self,
        generation: u64,
        transition_id: u64,
        ready: bool,
        locked: bool,
    ) {
        let Some(pending) = self.pending_trust_transition.as_ref() else {
            return;
        };
        if generation != self.trust_generation
            || !trust_projection_ack_matches(pending, generation, transition_id, ready, locked)
        {
            return;
        }
        let decision = pending.decision;
        self.pending_trust_transition = None;
        if decision == TrustLifecycleDecision::Lock {
            self.record_lifecycle_probe("lock_projection_ack");
            self.stop_restricted_sync().await;
            self.stop_normal_runtime_children().await;
            self.session_promoted = false;
            return;
        }
        self.record_lifecycle_probe("ready_projection_ack");
        let Some(session) = self.session.clone() else {
            return;
        };
        debug_assert!(
            self.restricted_sync.is_none(),
            "normal sync cannot start before restricted sync ownership is released"
        );
        self.start_incoming_verification_observer(session.clone())
            .await;
        self.spawn_sync_actor(session.clone()).await;
        record_verification_admission_event(verification_admission_event(
            "normal_sync_started",
            generation,
            transition_id,
        ));
        self.spawn_account_hydration(session.clone());
        self.start_recovery_observer(session.clone());
        self.start_session_change_observer(session);
        self.session_promoted = true;
        for event in std::mem::take(&mut self.pending_ready_events) {
            self.emit(event);
        }
    }

    /// Persist session credentials, mirroring the src-tauri flow: session
    /// JSON, saved-session index entry, last-session pointer — with rollback
    /// on partial failure.
    async fn persist_session(
        &self,
        session: &MatrixClientSession,
        key_id: &SessionKeyId,
    ) -> Result<PersistableMatrixSession, CoreFailure> {
        let store = self.store.clone();
        let session = session.clone();
        let key_id = key_id.clone();
        executor::spawn_blocking(move || {
            let backend = store.credential_backend();
            let persistable = session
                .persistable_session()
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            let json = persistable
                .to_json()
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            let stored = StoredMatrixSession::new(json);
            backend
                .save_matrix_session(&key_id, &stored)
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            if backend.remember_saved_session(&key_id).is_err() {
                let _ = backend.delete_matrix_session(&key_id);
                return Err(CoreFailure::StoreUnavailable);
            }
            if backend.save_last_session(&key_id).is_err() {
                let _ = backend.delete_matrix_session(&key_id);
                let _ = backend.forget_saved_session(&key_id);
                return Err(CoreFailure::StoreUnavailable);
            }
            Ok(persistable)
        })
        .await
        .unwrap_or(Err(CoreFailure::StoreUnavailable))
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
            self.clear_account_persistence(key_id).await;
        }
    }

    /// Remove all persisted material for one account: session JSON, saved
    /// session index entry, last-session pointer (only if it points at this
    /// account), unlock secret, and store/cache directories.
    async fn clear_account_persistence(&self, key_id: &SessionKeyId) {
        let store = self.store.clone();
        let key_id = key_id.clone();
        let _ = executor::spawn_blocking(move || {
            let backend = store.credential_backend();
            let _ = backend.delete_matrix_session(&key_id);
            let _ = backend.forget_saved_session(&key_id);
            match backend.load_last_session() {
                Ok(Some(last)) if last == key_id => {
                    let _ = backend.delete_last_session();
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = backend.delete_last_session();
                }
            }
            store.delete_account_credentials(&key_id);
        })
        .await;
    }

    /// Find the stored `SessionKeyId` for an account key (the user's Matrix
    /// ID). Checks the last-session pointer first, then the saved-session
    /// index. `Ok(None)` = no stored session; `Err(())` = store unreachable.
    async fn lookup_session_key_id(
        &self,
        account_key: &AccountKey,
    ) -> Result<Option<SessionKeyId>, ()> {
        let store = self.store.clone();
        let account_key = account_key.clone();
        executor::spawn_blocking(move || {
            let backend = store.credential_backend();
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
        })
        .await
        .unwrap_or(Err(()))
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

    async fn handle_probe_local_encryption_health(&self, request_id: RequestId) {
        let health = if let Some(key_id) = self.session_key_id.clone() {
            let store = self.store.clone();
            executor::spawn_blocking(move || store.probe_local_encryption_health(&key_id))
                .await
                .unwrap_or(koushi_state::LocalEncryptionHealth::Unavailable)
        } else {
            koushi_state::LocalEncryptionHealth::Unknown
        };
        self.send_actions(vec![AppAction::LocalEncryptionHealthChanged {
            request_id: request_id.sequence,
            health,
        }])
        .await;
        self.emit(CoreEvent::LocalEncryption(
            LocalEncryptionEvent::HealthChanged { health },
        ));
    }

    async fn handle_reset_local_data(&mut self, request_id: RequestId) {
        let Some(key_id) = self.session_key_id.take() else {
            self.send_actions(vec![AppAction::ResetLocalDataFailed {
                request_id: request_id.sequence,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.stop_current_session_runtime().await;

        drop(self.session.take());
        self.clear_account_persistence(&key_id).await;
        self.send_actions(vec![
            AppAction::ResetLocalDataCompleted {
                request_id: request_id.sequence,
            },
            AppAction::LogoutFinished,
        ])
        .await;
    }

    async fn send_actions(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.send(actions).await;
    }
}

fn method_discovery_is_current(
    generation: u64,
    current_generation: u64,
    serial: u64,
    current_serial: u64,
    has_session: bool,
) -> bool {
    has_session && generation == current_generation && serial == current_serial
}

fn retry_should_restart_method_discovery(
    session_promoted: bool,
    trust: Option<koushi_state::CurrentDeviceTrustState>,
    discovery_task_active: bool,
    discovery_failed: bool,
) -> bool {
    !session_promoted
        && (discovery_task_active || discovery_failed)
        && matches!(
            trust,
            Some(koushi_state::CurrentDeviceTrustState::Unverified)
        )
}

async fn wait_for_verification_method_discovery<F>(
    timeout: Duration,
    future: F,
) -> VerificationMethodDiscoveryResult
where
    F: Future<Output = koushi_state::VerificationGateState>,
{
    match executor::timeout(timeout, future).await {
        Err(_) => VerificationMethodDiscoveryResult::Failed(
            koushi_state::VerificationGateFailureKind::Timeout,
        ),
        Ok(gate) if gate.account_kind == koushi_state::VerificationAccountKind::Unknown => {
            VerificationMethodDiscoveryResult::Failed(
                koushi_state::VerificationGateFailureKind::Sdk,
            )
        }
        Ok(gate) => VerificationMethodDiscoveryResult::Discovered(gate),
    }
}

fn recovery_result_is_current(
    generation: u64,
    current_generation: u64,
    flow_id: u64,
    current_flow_id: u64,
    request_id: RequestId,
    current_request_id: RequestId,
    has_session: bool,
) -> bool {
    has_session
        && generation == current_generation
        && flow_id == current_flow_id
        && request_id == current_request_id
}

fn first_restricted_sync_is_current(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
) -> bool {
    has_session && !session_promoted && generation == current_generation
}

fn own_user_sas_recheck_is_current(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
    has_own_user_flow: bool,
    has_sas: bool,
) -> bool {
    generation == current_generation
        && has_session
        && !session_promoted
        && has_own_user_flow
        && !has_sas
}

fn active_own_user_sas_flow_for_restricted_sync(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
    own_flow_id: Option<u64>,
) -> Option<u64> {
    (generation == current_generation && has_session && !session_promoted)
        .then_some(own_flow_id)
        .flatten()
}

fn unknown_verification_gate() -> koushi_state::VerificationGateState {
    koushi_state::VerificationGateState {
        methods: Vec::new(),
        account_kind: koushi_state::VerificationAccountKind::Unknown,
        failure: None,
    }
}

#[cfg(test)]
fn should_discover_verification_methods(trust: koushi_state::CurrentDeviceTrustState) -> bool {
    matches!(
        trust,
        koushi_state::CurrentDeviceTrustState::Unknown
            | koushi_state::CurrentDeviceTrustState::Unverified
    )
}

fn sas_projection_action(own_user_flow: bool, flow_id: u64, emojis: Vec<SasEmoji>) -> AppAction {
    if own_user_flow {
        AppAction::GateSasPresented { flow_id, emojis }
    } else {
        AppAction::VerificationSasPresented {
            request_id: flow_id,
            emojis,
        }
    }
}

fn trace_room_route(stage: &'static str, command: &RoomCommand) {
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

fn trace_room_route_event(stage: &'static str, kind: &'static str, request_id: RequestId) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.account", stage)
            .field(DiagnosticField::token("operation", kind))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            )),
    );
}

fn trace_room_route_closed() {
    record(DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.account",
        "closed",
    ));
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

async fn account_hydration_actions_from_session(
    session: &MatrixClientSession,
) -> (Vec<AppAction>, Option<BTreeSet<String>>) {
    let mut actions = Vec::new();
    let mut ignored_user_ids = None;

    if let Some(action) = own_profile_action_from_session(session).await {
        actions.push(action);
    }
    if let Some(action) = local_user_aliases_action_from_session(session).await {
        actions.push(action);
    }
    if let Some(action) = ignored_user_ids_action_from_session(session).await {
        if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
            ignored_user_ids = Some(user_ids.clone());
        }
        actions.push(action);
    }

    (actions, ignored_user_ids)
}

async fn own_profile_action_from_session(session: &MatrixClientSession) -> Option<AppAction> {
    crate::executor::timeout(
        ACCOUNT_HYDRATION_TIMEOUT,
        koushi_sdk::get_own_profile(session),
    )
    .await
    .ok()?
    .ok()
    .map(map_matrix_own_profile)
    .map(|profile| AppAction::OwnProfileUpdated { profile })
}

async fn local_user_aliases_action_from_session(
    session: &MatrixClientSession,
) -> Option<AppAction> {
    crate::executor::timeout(
        ACCOUNT_HYDRATION_TIMEOUT,
        koushi_sdk::get_local_user_aliases(session),
    )
    .await
    .ok()?
    .ok()
    .map(|aliases| AppAction::LocalUserAliasesLoaded {
        aliases: aliases.aliases,
    })
}

async fn ignored_user_ids_action_from_session(session: &MatrixClientSession) -> Option<AppAction> {
    crate::executor::timeout(
        ACCOUNT_HYDRATION_TIMEOUT,
        koushi_sdk::get_ignored_user_list(session),
    )
    .await
    .ok()?
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
            true,
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
            if lower.contains("passphrase")
                || lower.contains("mac")
                || lower.contains("decrypt")
                || lower.contains("recovery key")
                || lower.contains("invalid key")
            {
                TrustOperationFailureKind::InvalidPassphrase
            } else if lower.contains("timeout") {
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

fn verification_gate_failure_kind(
    error: &koushi_sdk::E2eeTrustError,
) -> koushi_state::VerificationGateFailureKind {
    match classify_e2ee_trust_error(error) {
        TrustOperationFailureKind::Cancelled => {
            koushi_state::VerificationGateFailureKind::Cancelled
        }
        TrustOperationFailureKind::Mismatch => koushi_state::VerificationGateFailureKind::Mismatch,
        TrustOperationFailureKind::Network => koushi_state::VerificationGateFailureKind::Network,
        TrustOperationFailureKind::Forbidden => {
            koushi_state::VerificationGateFailureKind::Forbidden
        }
        TrustOperationFailureKind::Timeout => koushi_state::VerificationGateFailureKind::Timeout,
        TrustOperationFailureKind::InvalidPassphrase | TrustOperationFailureKind::Sdk => {
            koushi_state::VerificationGateFailureKind::Sdk
        }
    }
}

fn classify_e2ee_trust_auth_failure(error: &koushi_sdk::E2eeTrustError) -> AuthFailureKind {
    match classify_e2ee_trust_error(error) {
        TrustOperationFailureKind::Network => AuthFailureKind::Network,
        TrustOperationFailureKind::Forbidden => AuthFailureKind::Forbidden,
        TrustOperationFailureKind::Timeout => AuthFailureKind::Timeout,
        TrustOperationFailureKind::Cancelled
        | TrustOperationFailureKind::Mismatch
        | TrustOperationFailureKind::InvalidPassphrase
        | TrustOperationFailureKind::Sdk => AuthFailureKind::Sdk,
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

fn project_identity_reset_failed_event(
    request_id: u64,
    account_key: AccountKey,
    kind: TrustOperationFailureKind,
) -> Vec<CoreEvent> {
    vec![
        CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
            account_key: account_key.clone(),
            status: CrossSigningStatus::Failed { request_id, kind },
        }),
        CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state: IdentityResetState::Failed { request_id, kind },
        }),
    ]
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

fn classify_auth_error(error: &koushi_sdk::PasswordLoginError) -> AuthFailureKind {
    match error {
        koushi_sdk::PasswordLoginError::InvalidHomeserver(discovery_err) => {
            login_discovery_failure_kind(discovery_err)
        }
        koushi_sdk::PasswordLoginError::Sdk(message) => {
            if message.contains("401")
                || message.contains("403")
                || message.contains("M_FORBIDDEN")
                || message.contains("M_UNAUTHORIZED")
            {
                AuthFailureKind::Forbidden
            } else {
                AuthFailureKind::Sdk
            }
        }
        koushi_sdk::PasswordLoginError::Runtime(_)
        | koushi_sdk::PasswordLoginError::MissingSession
        | koushi_sdk::PasswordLoginError::Serialization(_) => AuthFailureKind::Sdk,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::{fs, path::Path};

    use futures_util::stream;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use tempfile::tempdir;
    use tokio::sync::{broadcast, mpsc, oneshot};

    use super::*;
    use crate::store::CredentialStoreBackend;

    #[test]
    fn scheduled_thread_message_content_preserves_the_thread_relation() {
        let content = build_scheduled_message_content(
            "scheduled thread body",
            Some("$thread-root:example.invalid"),
        )
        .expect("thread content should build");
        let value = serde_json::to_value(content).expect("content should serialize");

        assert_eq!(value["m.relates_to"]["rel_type"], "m.thread");
        assert_eq!(
            value["m.relates_to"]["event_id"],
            "$thread-root:example.invalid"
        );
    }

    #[test]
    fn own_user_sas_projects_gate_action_while_peer_sas_keeps_peer_projection() {
        let emojis = vec![
            SasEmoji {
                symbol: "x".into(),
                description: "opaque".into()
            };
            7
        ];
        assert!(
            matches!(sas_projection_action(true, 41, emojis.clone()), AppAction::GateSasPresented { flow_id: 41, emojis: projected } if projected == emojis)
        );
        assert!(
            matches!(sas_projection_action(false, 42, emojis.clone()), AppAction::VerificationSasPresented { request_id: 42, emojis: projected } if projected == emojis)
        );
    }

    #[test]
    fn method_discovery_rejects_stale_generation_serial_and_missing_session() {
        assert!(method_discovery_is_current(4, 4, 9, 9, true));
        assert!(!method_discovery_is_current(3, 4, 9, 9, true));
        assert!(!method_discovery_is_current(4, 4, 8, 9, true));
        assert!(!method_discovery_is_current(4, 4, 9, 9, false));
    }

    #[tokio::test]
    async fn verification_method_discovery_times_out_pending_sdk_work() {
        let result = wait_for_verification_method_discovery(
            Duration::from_millis(1),
            std::future::pending(),
        )
        .await;

        assert_eq!(
            result,
            VerificationMethodDiscoveryResult::Failed(
                koushi_state::VerificationGateFailureKind::Timeout
            )
        );
    }

    #[tokio::test]
    async fn verification_method_discovery_maps_known_and_unknown_gate_results() {
        let known = koushi_state::VerificationGateState {
            methods: vec![koushi_state::VerificationMethodCapability::RecoveryKey],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: None,
        };
        assert_eq!(
            wait_for_verification_method_discovery(
                Duration::from_secs(1),
                std::future::ready(known.clone()),
            )
            .await,
            VerificationMethodDiscoveryResult::Discovered(known)
        );
        assert_eq!(
            wait_for_verification_method_discovery(
                Duration::from_secs(1),
                std::future::ready(unknown_verification_gate()),
            )
            .await,
            VerificationMethodDiscoveryResult::Failed(
                koushi_state::VerificationGateFailureKind::Sdk
            )
        );
    }

    #[test]
    fn verification_method_discovery_retry_restarts_only_for_unverified_provisional_session() {
        assert!(retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            true,
            false,
        ));
        assert!(retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            false,
            true,
        ));
        assert!(!retry_should_restart_method_discovery(
            true,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            true,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unknown),
            true,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Verified),
            true,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            false,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false, None, true, true,
        ));
    }

    #[test]
    fn recovery_result_requires_current_generation_flow_request_and_session() {
        let current = test_request_id();
        let other = RequestId {
            connection_id: current.connection_id,
            sequence: current.sequence + 1,
        };

        assert!(recovery_result_is_current(
            4, 4, 9, 9, current, current, true
        ));
        assert!(!recovery_result_is_current(
            3, 4, 9, 9, current, current, true
        ));
        assert!(!recovery_result_is_current(
            4, 4, 8, 9, current, current, true
        ));
        assert!(!recovery_result_is_current(
            4, 4, 9, 9, other, current, true
        ));
        assert!(!recovery_result_is_current(
            4, 4, 9, 9, current, current, false
        ));
    }

    #[test]
    fn restricted_sync_attempt_starts_only_without_an_active_owner() {
        assert!(begin_restricted_sync_cursor_attempt(false));
        assert!(!begin_restricted_sync_cursor_attempt(true));
    }

    #[test]
    fn first_restricted_sync_ack_rejects_stale_torn_down_and_promoted_sessions() {
        assert!(first_restricted_sync_is_current(4, 4, true, false));
        assert!(!first_restricted_sync_is_current(3, 4, true, false));
        assert!(!first_restricted_sync_is_current(4, 4, false, false));
        assert!(!first_restricted_sync_is_current(4, 4, true, true));
        assert_eq!(unknown_verification_gate().methods, Vec::new());
        assert_eq!(
            unknown_verification_gate().account_kind,
            koushi_state::VerificationAccountKind::Unknown
        );
    }

    #[test]
    fn restricted_sync_rechecks_only_current_unstarted_own_user_flow() {
        assert!(own_user_sas_recheck_is_current(
            4, 4, true, false, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            3, 4, true, false, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, false, false, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, true, true, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, true, false, false, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, true, false, true, true
        ));
    }

    #[test]
    fn restricted_sync_diagnostics_require_current_own_user_flow() {
        assert_eq!(
            active_own_user_sas_flow_for_restricted_sync(4, 4, true, false, Some(73)),
            Some(73)
        );
        assert_eq!(
            active_own_user_sas_flow_for_restricted_sync(3, 4, true, false, Some(73)),
            None
        );
        assert_eq!(
            active_own_user_sas_flow_for_restricted_sync(4, 4, false, false, Some(73)),
            None
        );
        assert_eq!(
            active_own_user_sas_flow_for_restricted_sync(4, 4, true, true, Some(73)),
            None
        );
        assert_eq!(
            active_own_user_sas_flow_for_restricted_sync(4, 4, true, false, None),
            None
        );
    }

    #[test]
    fn sas_handle_adoption_is_classified_before_any_active_flow_side_effect() {
        let source = include_str!("account.rs");
        let body = source
            .split("    async fn store_sas_verification(")
            .nth(1)
            .expect("store_sas_verification should exist")
            .split("    async fn project_sas_state(")
            .next()
            .expect("project_sas_state should follow SAS handle adoption");

        let classify = body
            .find("resolve_sas_adoption(")
            .expect("SAS handle adoption must classify the incoming flow");
        let early_return = body
            .find("return;")
            .expect("replayed or conflicting active SAS flows must exit before adoption");
        assert!(
            classify < early_return,
            "the adoption decision must be made before its no-op return"
        );
        assert!(
            body.contains("koushi_sdk::cancel_sas_verification(&handle)"),
            "a distinct conflicting SAS handle must be explicitly rejected"
        );

        for side_effect in [
            "self.stop_sas_verification_observer().await",
            "self.sas_verification = Some",
            "self.start_sas_timeout(",
            "self.observe_sas_verification(",
            "koushi_sdk::accept_sas_verification(",
        ] {
            let side_effect = body
                .find(side_effect)
                .unwrap_or_else(|| panic!("missing expected adoption side effect: {side_effect}"));
            assert!(
                early_return < side_effect,
                "SAS adoption guard must precede side effect: {side_effect}"
            );
        }
    }

    #[test]
    fn sas_adoption_decision_adopts_once_and_rejects_replay_or_conflict() {
        assert_eq!(classify_sas_adoption(None, 41), SasAdoptionDecision::Adopt);
        assert_eq!(
            classify_sas_adoption(Some(41), 41),
            SasAdoptionDecision::Replay
        );
        assert_eq!(
            classify_sas_adoption(Some(41), 42),
            SasAdoptionDecision::Conflict
        );
    }

    #[tokio::test]
    async fn sas_replay_is_noop_but_conflict_runs_explicit_rejection() {
        let replay_rejections = Arc::new(AtomicU64::new(0));
        let replay = resolve_sas_adoption(Some(41), 41, {
            let replay_rejections = Arc::clone(&replay_rejections);
            move || async move {
                replay_rejections.fetch_add(1, Ordering::SeqCst);
                true
            }
        })
        .await;
        assert_eq!(replay, (SasAdoptionDecision::Replay, None));
        assert_eq!(replay_rejections.load(Ordering::SeqCst), 0);

        let conflict_rejections = Arc::new(AtomicU64::new(0));
        let conflict = resolve_sas_adoption(Some(41), 42, {
            let conflict_rejections = Arc::clone(&conflict_rejections);
            move || async move {
                conflict_rejections.fetch_add(1, Ordering::SeqCst);
                false
            }
        })
        .await;
        assert_eq!(conflict, (SasAdoptionDecision::Conflict, Some(false)));
        assert_eq!(conflict_rejections.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn at_least_once_incoming_transport_uses_target_and_flow_identity() {
        let active_target = VerificationTarget {
            user_id: "@alice:example.test".to_owned(),
            device_id: "ALICE".to_owned(),
        };
        let peer_collision = VerificationTarget {
            user_id: "@mallory:example.test".to_owned(),
            device_id: "MALLORY".to_owned(),
        };
        let device_collision = VerificationTarget {
            user_id: active_target.user_id.clone(),
            device_id: "ALICE-SECOND".to_owned(),
        };
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: Some((&active_target, "stable-flow")),
                    sas_active: false,
                    own_user_active: false,
                },
                &peer_collision,
                "stable-flow",
            ),
            IncomingVerificationRequestDecision::Conflict,
            "the same opaque flow ID from a different peer/device must be rejected",
        );
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: Some((&active_target, "stable-flow")),
                    sas_active: false,
                    own_user_active: false,
                },
                &device_collision,
                "stable-flow",
            ),
            IncomingVerificationRequestDecision::Conflict,
            "the same opaque flow ID from a different device must be rejected",
        );
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: Some((&active_target, "stable-flow")),
                    sas_active: false,
                    own_user_active: false,
                },
                &active_target,
                "stable-flow",
            ),
            IncomingVerificationRequestDecision::Replay,
        );
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: Some((&active_target, "stable-flow")),
                    sas_active: false,
                    own_user_active: false,
                },
                &active_target,
                "other-flow",
            ),
            IncomingVerificationRequestDecision::Conflict,
        );
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: None,
                    sas_active: false,
                    own_user_active: false,
                },
                &active_target,
                "new-flow",
            ),
            IncomingVerificationRequestDecision::Adopt,
        );
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: None,
                    sas_active: true,
                    own_user_active: false,
                },
                &active_target,
                "new-flow",
            ),
            IncomingVerificationRequestDecision::Conflict,
            "an active SAS continuation must continue to reject a new request",
        );
    }

    #[test]
    fn active_own_user_verification_conflicts_with_incoming_request() {
        let incoming_target = VerificationTarget {
            user_id: "@alice:example.test".to_owned(),
            device_id: "ALICE".to_owned(),
        };
        assert_eq!(
            classify_incoming_verification_request(
                IncomingVerificationActivity {
                    active_request: None,
                    sas_active: false,
                    own_user_active: true,
                },
                &incoming_target,
                "incoming-flow",
            ),
            IncomingVerificationRequestDecision::Conflict,
            "an own-user verification owns the shared continuation/observer slots",
        );
    }

    #[test]
    fn incoming_actor_admission_checks_own_user_before_replacing_runtime() {
        let source = include_str!("account.rs");
        let start = source
            .find("    async fn handle_incoming_verification_request(")
            .expect("incoming verification actor handler");
        let end = source[start..]
            .find("\n    async fn handle_set_presence(")
            .expect("end of incoming verification actor handler");
        let body = &source[start..start + end];

        let own_user_state = body
            .find("own_user_active: self.own_user_verification.is_some()")
            .expect("actor admission must include active own-user verification state");
        let decision_match = body
            .find("match decision")
            .expect("incoming admission decision");
        let cancel = body
            .find("koushi_sdk::cancel_verification_request(&handle).await")
            .expect("conflicting incoming request cancellation");
        let adopt_handle = body
            .find("self.verification_request = Some")
            .expect("incoming request handle adoption");
        let replace_observer = body
            .find("self.observe_verification_request(")
            .expect("incoming request observer adoption");

        assert!(
            own_user_state < decision_match,
            "own-user activity must feed admission"
        );
        assert!(
            cancel < adopt_handle,
            "conflict cancellation must precede handle adoption"
        );
        assert!(
            cancel < replace_observer,
            "conflict cancellation must precede observer adoption"
        );
    }

    #[test]
    fn incoming_verification_transport_rejects_stale_or_sessionless_messages() {
        assert!(incoming_verification_request_is_current(7, 7, true));
        assert!(!incoming_verification_request_is_current(6, 7, true));
        assert!(!incoming_verification_request_is_current(7, 7, false));
    }

    #[tokio::test]
    async fn incoming_verification_mailbox_send_is_stop_aware_when_full() {
        let (sender, mut receiver) = mpsc::channel(1);
        let (_first_stop_tx, mut first_stop_rx) = oneshot::channel();
        assert!(
            send_incoming_verification_message_until_stopped(&sender, 1_u8, &mut first_stop_rx,)
                .await,
            "the first ready delivery must fill the product mailbox"
        );
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let blocked_send = executor::spawn(async move {
            send_incoming_verification_message_until_stopped(&sender, 2, &mut stop_rx).await
        });
        tokio::task::yield_now().await;

        stop_tx.send(()).expect("request observer stop");
        let delivered = executor::timeout(Duration::from_millis(20), blocked_send)
            .await
            .expect("a stop request must interrupt the full-mailbox send")
            .expect("send task");
        assert!(
            !delivered,
            "a stopped observer must not report the blocked send as delivered"
        );
        assert_eq!(receiver.recv().await, Some(1));
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn incoming_verification_observer_join_has_a_bounded_abort_fallback() {
        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = koushi_sdk::restore_session(&persistable)
            .await
            .expect("synthetic session should restore");
        let mut observer = koushi_sdk::observe_incoming_verification_requests(&session).await;
        let receiver = observer
            .take_receiver()
            .expect("observer receiver is available once");
        let (stop_tx, _stop_rx) = oneshot::channel();
        let child = executor::spawn(async move {
            let _receiver = receiver;
            std::future::pending::<()>().await
        });
        let child_abort = child.abort_handle();
        let observation = IncomingVerificationObservation {
            stop_tx,
            task: child,
            observer,
        };
        let mut stop = executor::spawn(stop_incoming_verification_observation_with_timeout(
            observation,
            Duration::from_millis(1),
        ));

        let result = executor::timeout(Duration::from_millis(20), &mut stop).await;
        if result.is_err() {
            stop.abort();
            child_abort.abort();
        }
        assert!(
            result.is_ok(),
            "a nonresponsive observer must be aborted after a bounded join"
        );
    }

    #[test]
    fn unknown_trust_discovers_methods_without_becoming_verified() {
        assert!(should_discover_verification_methods(
            koushi_state::CurrentDeviceTrustState::Unknown
        ));
        assert!(should_discover_verification_methods(
            koushi_state::CurrentDeviceTrustState::Unverified
        ));
        assert!(!should_discover_verification_methods(
            koushi_state::CurrentDeviceTrustState::Verified
        ));
    }

    #[tokio::test]
    async fn actor_sas_settlement_emits_exactly_one_terminal_and_clears_runtime() {
        let diagnostic_start = koushi_diagnostics::snapshot().records.len();
        let cred_dir = tempdir().expect("credential tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );
        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _) = broadcast::channel(16);
        let handle = AccountActor::spawn(store, action_tx, event_tx, LinkPreviewContext::default());

        let cases = [
            SyntheticVerificationTerminal::Success,
            SyntheticVerificationTerminal::Cancelled(VerificationCancelReason::User),
            SyntheticVerificationTerminal::Cancelled(VerificationCancelReason::Mismatch),
            SyntheticVerificationTerminal::Failed(TrustOperationFailureKind::Timeout),
            SyntheticVerificationTerminal::Failed(TrustOperationFailureKind::Sdk),
        ];
        for (index, terminal) in cases.into_iter().enumerate() {
            let flow_id = index as u64 + 100;
            assert!(
                handle
                    .send(AccountMessage::ConfigureSyntheticVerification { flow_id })
                    .await
            );
            assert!(
                handle
                    .send(AccountMessage::SettleSyntheticVerification { flow_id, terminal })
                    .await
            );
            let actions = action_rx.recv().await.expect("one terminal action");
            assert_eq!(
                actions.len(),
                1,
                "flow {flow_id} must emit one terminal action"
            );
            let terminal_request_id = match (&terminal, actions.as_slice()) {
                (
                    SyntheticVerificationTerminal::Success,
                    [AppAction::VerificationCompleted { request_id }],
                )
                | (
                    SyntheticVerificationTerminal::Cancelled(_),
                    [AppAction::VerificationCancelled { request_id, .. }],
                )
                | (
                    SyntheticVerificationTerminal::Failed(_),
                    [AppAction::VerificationFailed { request_id, .. }],
                ) => *request_id,
                unexpected => panic!("unexpected terminal projection: {unexpected:?}"),
            };
            assert_eq!(terminal_request_id, flow_id);

            let (response, inspected) = oneshot::channel();
            assert!(
                handle
                    .send(AccountMessage::InspectVerificationRuntime { response })
                    .await
            );
            assert_eq!(
                inspected.await.expect("runtime inspection"),
                (false, false, false, false, false, false, false)
            );

            assert!(
                handle
                    .send(AccountMessage::SettleSyntheticVerification { flow_id, terminal })
                    .await
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(20), action_rx.recv())
                    .await
                    .is_err(),
                "stale terminal duplicated flow {flow_id}"
            );
        }
        let settled_flow_ids = koushi_diagnostics::snapshot().records[diagnostic_start..]
            .iter()
            .filter(|record| {
                record.event.source == "core.sas_verification" && record.event.stage == "settled"
            })
            .filter_map(|record| {
                record
                    .event
                    .fields
                    .iter()
                    .find_map(|field| (field.key == "flow_id").then_some(&field.value))
            })
            .filter_map(|value| match value {
                koushi_diagnostics::DiagnosticValue::Count(flow_id) => Some(*flow_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(settled_flow_ids, vec![100, 101, 102, 103, 104]);
        shutdown_and_ack(&handle).await;
    }

    async fn restore_media_test_session(
        server: &MatrixMockServer,
        data_dir: &Path,
    ) -> MatrixClientSession {
        let persisted = PersistableMatrixSession::from_json(
            &serde_json::json!({
                "homeserver": server.uri(),
                "access_token": "1234",
                "device_id": "AVATARCACHEDEVICE",
                "user_id": "@avatar-cache:localhost"
            })
            .to_string(),
        )
        .expect("synthetic Matrix session");
        let store_config = koushi_sdk::MatrixClientStoreConfig::new(
            data_dir.join("matrix-store"),
            koushi_sdk::MatrixClientStoreKey::new([41; 32]),
        )
        .with_cache_path(data_dir.join("matrix-cache"));

        koushi_sdk::restore_session_with_store(&persisted, Some(&store_config))
            .await
            .expect("restore media test session")
    }

    fn assert_directory_does_not_contain_plaintext(root: &Path, plaintext: &[u8]) {
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            for entry in fs::read_dir(path).expect("read test store directory") {
                let entry = entry.expect("read test store entry");
                let file_type = entry.file_type().expect("read test store entry type");
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    let bytes = fs::read(entry.path()).expect("read test store file");
                    assert!(
                        !bytes
                            .windows(plaintext.len())
                            .any(|window| window == plaintext),
                        "keyed SDK media store must not persist renderable avatar plaintext"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn avatar_download_survives_restart_and_offline_via_keyed_sdk_media_store() {
        let server = MatrixMockServer::new().await;
        server.mock_versions().ok().mount().await;
        server
            .mock_authed_media_download()
            .ok_image()
            .named("avatar fetched from network exactly once")
            .expect(1)
            .mount()
            .await;
        let data_dir = tempdir().expect("data tempdir");
        let mxc_uri = "mxc://localhost/persisted-avatar";

        let online_session = restore_media_test_session(&server, data_dir.path()).await;
        let online = download_avatar_thumbnail(&online_session, mxc_uri)
            .await
            .expect("online avatar fetch");
        let AvatarThumbnailState::Ready { source_url, .. } = online else {
            panic!("online avatar should be ready");
        };
        assert!(source_url.starts_with("koushi-thumbnail://"));
        assert!(!source_url.starts_with("file://"));
        drop(online_session);
        clear_renderable_thumbnail_cache();

        let offline_session = restore_media_test_session(&server, data_dir.path()).await;
        let offline = download_avatar_thumbnail(&offline_session, mxc_uri)
            .await
            .expect("cached avatar should load without a second network request");
        let AvatarThumbnailState::Ready { source_url, .. } = offline else {
            panic!("offline cached avatar should be ready");
        };
        assert!(source_url.starts_with("koushi-thumbnail://"));
        assert!(!source_url.starts_with("file://"));
        assert!(!data_dir.path().join("avatar_thumbnails").exists());
        assert_directory_does_not_contain_plaintext(data_dir.path(), b"binaryjpegfullimagedata");
    }

    #[tokio::test]
    async fn uncached_avatar_offline_preserves_network_failure() {
        let server = MatrixMockServer::new().await;
        server.mock_versions().ok().mount().await;
        let data_dir = tempdir().expect("data tempdir");
        let session = restore_media_test_session(&server, data_dir.path()).await;

        assert_eq!(
            download_avatar_thumbnail(&session, "mxc://localhost/uncached-avatar").await,
            Err(AvatarThumbnailFailureKind::Network)
        );
        assert!(!data_dir.path().join("avatar_thumbnails").exists());
    }

    #[test]
    fn account_trace_preserves_typed_request_fields_without_environment_switch() {
        trace_account_request(
            "test_account_typed_fields",
            RequestId {
                connection_id: RuntimeConnectionId(7),
                sequence: 11,
            },
            "restore_session",
        );
        let records = koushi_diagnostics::snapshot()
            .records
            .into_iter()
            .filter(|record| record.event.stage == "test_account_typed_fields")
            .collect::<Vec<_>>();
        assert_eq!(
            records.len(),
            1,
            "one account request must produce one collector event"
        );
        let record = &records[0];
        assert_eq!(record.event.source, "core.account");
        assert!(
            record
                .event
                .fields
                .iter()
                .any(|field| field.key == "request_id")
        );
        assert!(
            record
                .event
                .fields
                .iter()
                .any(|field| field.key == "action")
        );
    }

    #[test]
    fn verification_admission_diagnostic_records_and_writes_stderr() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "account::tests::verification_admission_diagnostic_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("verification admission diagnostic child should run");
        assert!(output.status.success(), "child failed: {output:?}");

        let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
        assert!(stderr.contains(
            "[koushi] core.verification_admission stage=trust_read_finished generation=7 transition_id=0 trust=verified elapsed_ms=42"
        ));
        assert!(!stderr.contains('@'));
        assert!(!stderr.contains("access_token"));

        let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
        let snapshot: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("child should print one JSON snapshot"),
        )
        .expect("child output should be a JSON snapshot");
        assert!(snapshot["records"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                record["event"]["source"] == "core.verification_admission"
                    && record["event"]["stage"] == "trust_read_finished"
            })
        }));
    }

    #[test]
    #[ignore]
    fn verification_admission_diagnostic_child() {
        record_verification_admission_event(
            DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "core.verification_admission",
                "trust_read_finished",
            )
            .field(DiagnosticField::count("generation", 7))
            .field(DiagnosticField::count("transition_id", 0))
            .field(DiagnosticField::token("trust", "verified"))
            .field(DiagnosticField::milliseconds("elapsed_ms", 42)),
        );
        println!(
            "{}",
            serde_json::to_string(&koushi_diagnostics::snapshot())
                .expect("diagnostic snapshot should serialize")
        );
    }

    #[test]
    fn verification_method_discovery_diagnostic_records_and_writes_stderr() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "account::tests::verification_method_discovery_diagnostic_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("verification method discovery diagnostic child should run");
        assert!(output.status.success(), "child failed: {output:?}");

        let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
        assert!(stderr.contains(
            "[koushi] core.verification_method_discovery stage=finished generation=7 serial=11 outcome=failed elapsed_ms=42"
        ));

        let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
        let snapshot: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("child should print one JSON snapshot"),
        )
        .expect("child output should be a JSON snapshot");
        assert!(snapshot["records"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                record["event"]["source"] == "core.verification_method_discovery"
                    && record["event"]["stage"] == "finished"
            })
        }));
    }

    #[test]
    #[ignore]
    fn verification_method_discovery_diagnostic_child() {
        record_verification_method_discovery_event(
            verification_method_discovery_event("finished", 7, 11)
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::milliseconds("elapsed_ms", 42)),
        );
        println!(
            "{}",
            serde_json::to_string(&koushi_diagnostics::snapshot())
                .expect("diagnostic snapshot should serialize")
        );
    }

    #[test]
    fn sas_verification_tokens_are_closed_and_private_safe() {
        use koushi_sdk::MatrixSasState as SasState;
        use koushi_sdk::MatrixVerificationCancelKind as CancelKind;
        use koushi_sdk::MatrixVerificationRequestState as RequestState;

        assert_eq!(
            verification_request_state_token(&RequestState::Created),
            "created"
        );
        assert_eq!(
            verification_request_state_token(&RequestState::Requested),
            "requested"
        );
        assert_eq!(
            verification_request_state_token(&RequestState::Ready),
            "ready"
        );
        assert_eq!(
            verification_request_state_token(&RequestState::Done),
            "done"
        );
        assert_eq!(
            verification_request_state_token(&RequestState::Cancelled {
                kind: CancelKind::Timeout,
                cancelled_by_us: false,
            }),
            "cancelled"
        );
        assert_eq!(
            verification_request_state_token(&RequestState::UnsupportedMethod),
            "unsupported_method"
        );

        let cancel_kinds = [
            (CancelKind::UnknownMethod, "unknown_method"),
            (CancelKind::KeyMismatch, "key_mismatch"),
            (CancelKind::User, "user"),
            (CancelKind::Timeout, "timeout"),
            (CancelKind::AcceptedElsewhere, "accepted_elsewhere"),
            (CancelKind::Other, "other"),
        ];
        for (kind, token) in cancel_kinds {
            assert_eq!(verification_cancel_kind_token(kind), token);
        }

        let sas_states = [
            (SasState::Created, "created"),
            (SasState::Started, "started"),
            (SasState::Accepted, "accepted"),
            (
                SasState::SasPresented { emojis: Vec::new() },
                "sas_presented",
            ),
            (SasState::Confirmed, "confirmed"),
            (SasState::Done, "done"),
            (
                SasState::Cancelled {
                    kind: CancelKind::Timeout,
                    cancelled_by_us: false,
                },
                "cancelled",
            ),
            (SasState::UnsupportedShortAuth, "unsupported_short_auth"),
        ];
        for (state, token) in sas_states {
            assert_eq!(sas_state_token(&state), token);
        }

        let failure_kinds = [
            (TrustOperationFailureKind::Cancelled, "cancelled"),
            (TrustOperationFailureKind::Mismatch, "mismatch"),
            (
                TrustOperationFailureKind::InvalidPassphrase,
                "invalid_passphrase",
            ),
            (TrustOperationFailureKind::Network, "network"),
            (TrustOperationFailureKind::Forbidden, "forbidden"),
            (TrustOperationFailureKind::Timeout, "timeout"),
            (TrustOperationFailureKind::Sdk, "sdk"),
        ];
        for (kind, token) in failure_kinds {
            assert_eq!(trust_failure_token(kind), token);
        }

        assert_eq!(
            verification_terminal_token(VerificationTerminal::Success),
            "success"
        );
        assert_eq!(
            verification_terminal_token(VerificationTerminal::Cancelled(
                VerificationCancelReason::User,
            )),
            "cancelled"
        );
        assert_eq!(
            verification_terminal_token(VerificationTerminal::Failed(
                TrustOperationFailureKind::Timeout,
            )),
            "failed"
        );
    }

    #[test]
    fn sas_cancel_diagnostic_contains_only_closed_private_safe_fields() {
        use koushi_sdk::MatrixSasState as SasState;
        use koushi_sdk::MatrixVerificationCancelKind as CancelKind;

        let cancelled = sas_state_changed_event(
            41,
            &SasState::Cancelled {
                kind: CancelKind::Timeout,
                cancelled_by_us: false,
            },
        );
        assert_eq!(
            koushi_diagnostics::format_event(&cancelled),
            "stage=sas_state_changed flow_id=41 state=cancelled cancel_kind=timeout cancelled_by_us=false"
        );

        let accepted = sas_state_changed_event(42, &SasState::Accepted);
        assert_eq!(
            koushi_diagnostics::format_event(&accepted),
            "stage=sas_state_changed flow_id=42 state=accepted"
        );
    }

    #[tokio::test]
    async fn own_user_sas_start_helper_traces_started_pending_and_failed_results() {
        let diagnostic_start = koushi_diagnostics::snapshot().records.len();

        assert_eq!(
            run_own_user_sas_start(211, "request_ready", async {
                Ok::<_, koushi_sdk::E2eeTrustError>(Some(7_u8))
            })
            .await
            .expect("started result"),
            Some(7)
        );
        assert_eq!(
            run_own_user_sas_start(212, "initial", async {
                Ok::<Option<u8>, koushi_sdk::E2eeTrustError>(None)
            })
            .await
            .expect("pending result"),
            None
        );
        assert!(
            run_own_user_sas_start(213, "restricted_sync", async {
                Err::<Option<u8>, _>(koushi_sdk::E2eeTrustError::Sdk(
                    "private SDK detail".to_owned(),
                ))
            })
            .await
            .is_err()
        );

        let records = koushi_diagnostics::snapshot().records;
        let events = records[diagnostic_start..]
            .iter()
            .filter(|record| record.event.source == "core.sas_verification")
            .map(|record| koushi_diagnostics::format_event(&record.event))
            .collect::<Vec<_>>();
        assert_eq!(
            events,
            vec![
                "stage=sas_start_attempted flow_id=211 source=request_ready",
                "stage=sas_start_finished flow_id=211 source=request_ready outcome=started",
                "stage=sas_start_attempted flow_id=212 source=initial",
                "stage=sas_start_finished flow_id=212 source=initial outcome=pending",
                "stage=sas_start_attempted flow_id=213 source=restricted_sync",
                "stage=sas_start_finished flow_id=213 source=restricted_sync outcome=failed failure_kind=sdk",
            ]
        );
        assert!(!events.join(" ").contains("private SDK detail"));
    }

    #[test]
    fn restricted_sync_failure_streak_reports_once_and_resets_after_success() {
        let mut failure_reported = false;
        assert!(should_report_restricted_sync_failure(
            &mut failure_reported,
            false
        ));
        assert!(!should_report_restricted_sync_failure(
            &mut failure_reported,
            false
        ));
        assert!(!should_report_restricted_sync_failure(
            &mut failure_reported,
            true
        ));
        assert!(should_report_restricted_sync_failure(
            &mut failure_reported,
            false
        ));
        assert!(!should_report_restricted_sync_failure(
            &mut failure_reported,
            false
        ));
    }

    #[test]
    fn sas_verification_diagnostic_records_and_writes_stderr() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "account::tests::sas_verification_diagnostic_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("SAS verification diagnostic child should run");
        assert!(output.status.success(), "child failed: {output:?}");

        let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
        assert!(stderr.contains(
            "[koushi] core.sas_verification stage=request_state_changed flow_id=41 state=cancelled cancel_kind=timeout cancelled_by_us=false"
        ));

        let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
        let snapshot: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("child should print one JSON snapshot"),
        )
        .expect("child output should be a JSON snapshot");
        assert!(snapshot["records"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                record["event"]["source"] == "core.sas_verification"
                    && record["event"]["stage"] == "request_state_changed"
            })
        }));
    }

    #[test]
    #[ignore]
    fn sas_verification_diagnostic_child() {
        record_sas_verification_event(
            sas_verification_event("request_state_changed", 41)
                .field(DiagnosticField::token("state", "cancelled"))
                .field(DiagnosticField::token("cancel_kind", "timeout"))
                .field(DiagnosticField::boolean("cancelled_by_us", false)),
        );
        println!(
            "{}",
            serde_json::to_string(&koushi_diagnostics::snapshot())
                .expect("diagnostic snapshot should serialize")
        );
    }

    #[test]
    fn event_cache_repair_diagnostic_runs_without_trace_environment() {
        let child = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .arg("--exact")
        .arg(concat!(
            "account::tests::",
            "event_cache_repair_diagnostic_records_without_trace_environment"
        ))
        .arg("--ignored")
        .arg("--nocapture")
        .env_remove("KOUSHI_TIMELINE_ITEM_TRACE")
        .env_remove("KOUSHI_SUBSCRIBE_TRACE")
        .status()
        .expect("env-unset event-cache-repair child should start");
        assert!(
            child.success(),
            "env-unset event-cache-repair child failed: {child}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn event_cache_repair_diagnostic_records_without_trace_environment() {
        assert!(std::env::var_os("KOUSHI_TIMELINE_ITEM_TRACE").is_none());
        assert!(std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_none());

        let cred_dir = tempdir().expect("credential tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let (handle, _action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let synthetic_room_id = "!synthetic-room:example.invalid";
        let synthetic_event_id = "$synthetic-event:example.invalid";
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(17),
            sequence: 23,
        };
        let (response_tx, response_rx) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::EnsureRoomEventCached {
                    request_id,
                    room_id: synthetic_room_id.to_owned(),
                    event_id: synthetic_event_id.to_owned(),
                    response_tx,
                })
                .await
        );
        response_rx.await.expect("cache-repair response");

        let records = koushi_diagnostics::snapshot().records;
        let repair = records
            .iter()
            .rev()
            .find(|record| {
                record.event.source == "core.event_cache_repair"
                    && record.event.stage == "skip"
                    && record.event.fields.iter().any(|field| {
                        field.key == "reason"
                            && field.value
                                == koushi_diagnostics::DiagnosticValue::Token("no_session")
                    })
            })
            .expect("event-cache repair should be collected without trace environment");
        assert_eq!(repair.event.source, "core.event_cache_repair");
        assert_eq!(repair.event.stage, "skip");
        assert_eq!(
            repair.event.fields,
            vec![
                koushi_diagnostics::DiagnosticField::request_id("request_id", 17, 23),
                koushi_diagnostics::DiagnosticField::token("outcome", "skipped"),
                koushi_diagnostics::DiagnosticField::token("reason", "no_session"),
            ]
        );

        let serialized = serde_json::to_string(&repair.event)
            .expect("event-cache repair event should serialize for privacy assertions");
        for forbidden in [
            synthetic_room_id,
            synthetic_event_id,
            "synthetic-body-value",
            "https://example.invalid/synthetic",
            "/tmp/synthetic-path",
            "raw sdk error: synthetic",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized event must not contain forbidden diagnostic data: {forbidden}"
            );
        }
    }

    #[test]
    fn incoming_verification_flow_ids_use_reserved_internal_namespace() {
        let request_id = incoming_verification_request_id(INCOMING_VERIFICATION_FLOW_ID_BASE);

        assert_eq!(request_id.connection_id, RuntimeConnectionId(0));
        assert_eq!(request_id.sequence, INCOMING_VERIFICATION_FLOW_ID_BASE);
    }

    #[test]
    fn scheduled_dispatch_targets_its_origin_session() {
        let origin = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "ALICE".to_owned(),
        };
        let switched = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@bob:example.test".to_owned(),
            device_id: "BOB".to_owned(),
        };

        assert!(scheduled_dispatch_targets_active_session(
            Some(&origin),
            &origin
        ));
        assert!(!scheduled_dispatch_targets_active_session(
            Some(&switched),
            &origin
        ));
        assert!(!scheduled_dispatch_targets_active_session(None, &origin));
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

    #[tokio::test]
    async fn logout_cleanup_is_bounded_and_ordered_before_persistence_removal() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let baseline_files = recursive_file_count(data_dir.path());
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false, true],
            })
            .await;
        let request_id = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        let files_before_logout = recursive_file_count(data_dir.path());
        assert!(files_before_logout > baseline_files);

        handle
            .send(AccountMessage::RejectProvisionalSession { request_id })
            .await;
        while probe_rx.recv().await != Some("session_store_close_retrying") {}
        assert_eq!(recursive_file_count(data_dir.path()), files_before_logout);
        assert_no_logout_finished(&mut action_rx);

        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
        assert_eq!(probe_rx.recv().await, Some("session_store_closed"));
        assert_eq!(probe_rx.recv().await, Some("session_persistence_deleted"));
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LogoutFinished])
        ) {}
        assert_eq!(recursive_file_count(data_dir.path()), baseline_files);
        loop {
            if let CoreEvent::Account(AccountEvent::LoggedOut {
                request_id: terminal,
                ..
            }) = event_rx.recv().await.expect("logout event")
            {
                assert_eq!(terminal, request_id);
                break;
            }
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn shutdown_quiesces_provisional_tasks_and_releases_session_without_logout_terminal() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        shutdown_and_ack(&handle).await;
        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        assert!(tokens.contains(&"trust_observer_terminated"));
        assert!(tokens.contains(&"restricted_sync_terminated"));
        assert!(tokens.contains(&"current_session_released"));
        assert_no_logout_finished(&mut action_rx);
        while let Ok(event) = event_rx.try_recv() {
            assert!(!matches!(
                event,
                CoreEvent::Account(AccountEvent::LoggedOut { .. })
            ));
        }
    }

    #[tokio::test]
    async fn shutdown_quiesces_promoted_children_before_releasing_session() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        while probe_rx.try_recv().is_ok() {}
        shutdown_and_ack(&handle).await;
        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        assert!(tokens.contains(&"trust_observer_terminated"));
        assert!(tokens.contains(&"shutdown_stop_sync_actor"));
        assert!(tokens.contains(&"shutdown_clear_room_session"));
        assert_eq!(tokens.last(), Some(&"current_session_released"));
    }

    #[tokio::test]
    async fn shutdown_aborts_pending_teardown_retry_and_releases_held_sessions_without_terminal() {
        let first_homeserver = spawn_quarantine_password_server();
        let second_homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        let request_id = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver: first_homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false; 8],
            })
            .await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: RequestId {
                    connection_id: crate::ids::RuntimeConnectionId(4),
                    sequence: 2,
                },
                request: LoginRequest {
                    homeserver: second_homeserver,
                    username: "replacement".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while probe_rx.recv().await != Some("session_store_close_retrying") {}
        shutdown_and_ack(&handle).await;
        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        assert!(tokens.contains(&"teardown_retry_terminated"));
        assert!(tokens.contains(&"pending_teardown_sessions_released"));
        assert_no_logout_finished(&mut action_rx);
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

    #[test]
    fn authentication_completion_installs_quarantine_before_ready_side_effects() {
        let source = include_str!("account.rs");
        let password = source
            .split("async fn handle_login_password")
            .nth(1)
            .and_then(|body| body.split("async fn handle_restore_session").next())
            .expect("password handler");
        let before_success = password
            .split("AppAction::LoginSucceeded")
            .next()
            .expect("password pre-success body");
        assert!(!before_success.contains("persist_session("));
        assert!(!before_success.contains("spawn_sync_actor("));
        assert!(before_success.contains("install_provisional_session"));

        let restore = source
            .split("async fn restore_account")
            .nth(1)
            .and_then(|body| body.split("async fn").next())
            .expect("restore handler");
        let before_restore_success = restore
            .split("AppAction::RestoreSessionSucceeded")
            .next()
            .expect("restore pre-success body");
        assert!(!before_restore_success.contains("spawn_sync_actor("));
        assert!(before_restore_success.contains("install_provisional_session"));
    }

    #[test]
    fn trust_lifecycle_is_generation_safe_and_fail_closed() {
        use koushi_state::CurrentDeviceTrustState::{Unknown, Unverified, Verified};

        assert_eq!(
            trust_lifecycle_decision(4, 5, false, Verified),
            TrustLifecycleDecision::IgnoreStale
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, false, Unknown),
            TrustLifecycleDecision::StayGated
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, false, Unverified),
            TrustLifecycleDecision::StayGated
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, false, Verified),
            TrustLifecycleDecision::Promote
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, true, Unverified),
            TrustLifecycleDecision::Lock
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, true, Unknown),
            TrustLifecycleDecision::Lock
        );
    }

    #[tokio::test]
    async fn password_quarantine_persists_no_credentials_and_restart_is_signed_out() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture-user".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: Some("Quarantine Test".to_owned()),
                    },
                }))
                .await
        );
        let actions = action_rx.recv().await.expect("provisional login action");
        assert!(matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ));

        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(
            backend
                .load_last_session()
                .expect("last pointer read")
                .is_none()
        );
        assert!(
            backend
                .load_saved_sessions()
                .expect("saved index read")
                .sessions()
                .is_empty()
        );

        let _ = handle.send(AccountMessage::Shutdown).await;
        let (restarted, mut restarted_actions, _events) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        assert!(
            restarted
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession { request_id }
                ))
                .await
        );
        assert!(matches!(
            restarted_actions.recv().await.as_deref(),
            Some([AppAction::RestoreSessionNotFound])
        ));
        let _ = restarted.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn oidc_completion_installs_only_a_provisional_quarantined_session() {
        let homeserver = spawn_quarantine_password_server();
        let login_session = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver: homeserver.clone(),
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("OIDC Quarantine Test".to_owned()),
            },
            None,
        )
        .await
        .expect("fixture login");

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (_trust_tx, trust_rx) = mpsc::unbounded_channel();
        let updates = futures_util::stream::unfold(trust_rx, |mut rx| async move {
            rx.recv().await.map(|trust| (trust, rx))
        });
        assert!(
            handle
                .send(AccountMessage::ConfigureTrustObservation {
                    observation: koushi_sdk::CurrentDeviceTrustObservation {
                        current: koushi_state::CurrentDeviceTrustState::Unknown,
                        updates: Box::pin(updates),
                    },
                })
                .await
        );
        let start_request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::ConfigureOidcCompletion {
                    start_request_id,
                    homeserver: homeserver.clone(),
                    session: login_session,
                })
                .await
        );
        let completion_request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(41),
            sequence: 7,
        };
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::CompleteOidcLogin {
                    request_id: completion_request_id,
                    callback_url: "http://127.0.0.1/callback?code=fixture&state=fixture".to_owned(),
                },))
                .await
        );
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::AuthenticationStarted {
                attempt_id,
                homeserver: projected_homeserver,
            }]) if *attempt_id == LoginAttemptId::new(41, 7)
                && projected_homeserver == &homeserver
        ));
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { attempt_id, .. }])
                if *attempt_id == LoginAttemptId::new(41, 7)
        ));
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );

        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(backend.load_last_session().expect("pointer read").is_none());
        assert!(
            backend
                .load_saved_sessions()
                .expect("index read")
                .sessions()
                .is_empty()
        );
        assert!(
            executor::timeout(Duration::from_millis(100), async {
                loop {
                    match event_rx.recv().await.expect("event stream") {
                        CoreEvent::Account(AccountEvent::LoggedIn { .. }) | CoreEvent::Sync(_) => {
                            return;
                        }
                        _ => {}
                    }
                }
            })
            .await
            .is_err(),
            "OIDC completion escaped quarantine before Verified"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn verified_warm_restore_skips_restricted_and_full_state_preparation() {
        let diagnostic_start = koushi_diagnostics::snapshot().records.len();
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}

        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, false),
            "authoritative Verified restore must not start restricted or promotion sync"
        );

        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, true),
            "normal sync must be the sole owner after Ready projection acknowledgement"
        );
        let snapshot = koushi_diagnostics::snapshot();
        let stages = snapshot.records[diagnostic_start..]
            .iter()
            .filter(|record| record.event.source == "core.verification_admission")
            .map(|record| record.event.stage)
            .collect::<Vec<_>>();
        let mut remaining = stages.as_slice();
        for expected in [
            "restricted_catch_up_skipped",
            "ready_projection_dispatched",
            "normal_sync_started",
        ] {
            let index = remaining
                .iter()
                .position(|stage| *stage == expected)
                .unwrap_or_else(|| {
                    panic!("missing ordered admission stage {expected}: {stages:?}")
                });
            remaining = &remaining[index + 1..];
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn verified_offline_warm_restore_reaches_ready_without_network_catch_up() {
        let (homeserver, offline) = spawn_controllable_quarantine_password_server();
        let login = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("Offline Restore Test".to_owned()),
            },
            None,
        )
        .await
        .expect("fixture login");
        let key_id = session_key_id_from_info(&login.info);
        let stored = StoredMatrixSession::new(
            login
                .persistable_session()
                .expect("persistable")
                .to_json()
                .expect("json"),
        );
        drop(login);

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        backend
            .save_matrix_session(&key_id, &stored)
            .expect("session seed");
        backend.remember_saved_session(&key_id).expect("index seed");
        backend.save_last_session(&key_id).expect("pointer seed");

        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession {
                    request_id: test_request_id(),
                },
            ))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::RestoreSessionSucceeded(_)])
        ) {}
        offline.store(true, std::sync::atomic::Ordering::SeqCst);

        executor::timeout(
            Duration::from_secs(1),
            acknowledge_next_verified_projection(&handle, &mut action_rx),
        )
        .await
        .expect("offline verified restore must reach Ready without network catch-up");
        assert_eq!(inspect_sync_owners(&handle).await, (false, false, true));
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    async fn login_gated_actor() -> (AccountActorHandle, mpsc::Receiver<Vec<AppAction>>) {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir").keep();
        let data_dir = tempdir().expect("tempdir").keep();
        let (handle, mut action_rx, _event_rx) = spawn_actor_with_dirs(&cred_dir, &data_dir);
        let updates = futures_util::stream::pending();
        handle
            .send(AccountMessage::ConfigureTrustObservation {
                observation: koushi_sdk::CurrentDeviceTrustObservation {
                    current: koushi_state::CurrentDeviceTrustState::Unknown,
                    updates: Box::pin(updates),
                },
            })
            .await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        (handle, action_rx)
    }

    #[tokio::test]
    async fn recovery_proof_success_enters_shared_authoritative_promotion_path() {
        let (handle, mut action_rx) = login_gated_actor().await;
        let flow_id = 81;
        let request_id = incoming_verification_request_id(flow_id);
        handle
            .send(AccountMessage::ConfigureSyntheticRecoveryTask {
                flow_id,
                pending: false,
            })
            .await;
        let (download_release, download) = oneshot::channel();
        handle
            .send(AccountMessage::ConfigureRecoveryDownload {
                completion: download,
            })
            .await;
        download_release
            .send(true)
            .expect("release recovery download");
        handle
            .send(AccountMessage::RecoveryFinished {
                generation: 2,
                flow_id,
                request_id,
                result: Ok(()),
            })
            .await;
        let _ = inspect_session_runtime(&handle).await;
        let _ = inspect_session_runtime(&handle).await;
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn invalid_recovery_terminal_stays_gated_without_normal_runtime() {
        let (handle, mut action_rx) = login_gated_actor().await;
        let flow_id = 82;
        let request_id = incoming_verification_request_id(flow_id);
        handle
            .send(AccountMessage::ConfigureSyntheticRecoveryTask {
                flow_id,
                pending: false,
            })
            .await;
        handle
            .send(AccountMessage::RecoveryFinished {
                generation: 2,
                flow_id,
                request_id,
                result: Err(koushi_sdk::E2eeRecoveryError::Sdk(
                    "invalid fixture secret".to_owned(),
                )),
            })
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::E2eeRecoveryFailed { .. }])
        ) {}
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn own_user_sas_proof_success_enters_shared_authoritative_promotion_path() {
        let (handle, mut action_rx) = login_gated_actor().await;
        let flow_id = 83;
        handle
            .send(AccountMessage::ConfigureSyntheticVerification { flow_id })
            .await;
        handle
            .send(AccountMessage::SettleSyntheticVerification {
                flow_id,
                terminal: SyntheticVerificationTerminal::Success,
            })
            .await;
        let _ = inspect_session_runtime(&handle).await;
        let _ = inspect_session_runtime(&handle).await;
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn verification_to_normal_sync_handoff_has_one_owner() {
        let diagnostic_start = koushi_diagnostics::snapshot().records.len();
        let (handle, mut action_rx) = login_gated_actor().await;
        assert!(
            koushi_diagnostics::snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "core.verification_admission"
                        && record.event.stage == "restricted_catch_up_started"
                }),
            "gated admission must diagnose restricted sync ownership start"
        );
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        assert_eq!(inspect_sync_owners(&handle).await, (true, false, false));
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, false),
            "restricted owner must stop before Ready projection acknowledgement"
        );
        assert_eq!(probe_rx.try_recv(), Ok("restricted_sync_terminated"));
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, true),
            "normal sync must be the only owner after Ready acknowledgement"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn provisional_rejection_deletes_keyed_store_before_signed_out_ack() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
        let baseline_files = recursive_file_count(data_dir.path());
        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture-user".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: Some("Quarantine Test".to_owned()),
                    },
                }))
                .await
        );
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ));
        assert!(
            recursive_file_count(data_dir.path()) > baseline_files,
            "keyed store was not created"
        );

        assert!(
            handle
                .send(AccountMessage::RejectProvisionalSession { request_id })
                .await
        );
        loop {
            let actions = action_rx.recv().await.expect("rejection action");
            if matches!(actions.as_slice(), [AppAction::LogoutFinished]) {
                assert_eq!(
                    probe_rx.try_recv(),
                    Ok("trust_observer_terminated"),
                    "LogoutFinished preceded trust-observer termination"
                );
                assert_eq!(
                    probe_rx.try_recv(),
                    Ok("restricted_sync_terminated"),
                    "LogoutFinished preceded restricted-sync termination"
                );
                assert_eq!(
                    recursive_file_count(data_dir.path()),
                    baseline_files,
                    "SignedOut ack preceded keyed-store deletion"
                );
                break;
            }
        }
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(backend.load_last_session().expect("pointer read").is_none());
        assert!(
            backend
                .load_saved_sessions()
                .expect("index read")
                .sessions()
                .is_empty()
        );
        shutdown_and_ack(&handle).await;
        let (restarted, mut restarted_actions, _restarted_events) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let restore_id = RequestId {
            connection_id: RuntimeConnectionId(19),
            sequence: 1,
        };
        assert!(
            restarted
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession {
                        request_id: restore_id,
                    },
                ))
                .await
        );
        assert!(matches!(
            restarted_actions.recv().await.as_deref(),
            Some([AppAction::RestoreSessionNotFound])
        ));
        shutdown_and_ack(&restarted).await;
    }

    #[tokio::test]
    async fn teardown_close_failure_retries_without_early_ack_and_preserves_request_correlation() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false, true],
            })
            .await;
        let original = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: original,
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: Some("Teardown Retry Test".to_owned()),
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        handle
            .send(AccountMessage::RejectProvisionalSession {
                request_id: original,
            })
            .await;
        while probe_rx.recv().await != Some("session_store_close_retrying") {}
        assert_no_logout_finished(&mut action_rx);

        let later = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(77),
            sequence: 2,
        };
        handle
            .send(AccountMessage::RejectProvisionalSession { request_id: later })
            .await;
        loop {
            if let CoreEvent::OperationFailed {
                request_id,
                failure,
            } = event_rx.recv().await.expect("failure event")
                && request_id == later
            {
                assert_eq!(failure, CoreFailure::SessionRequired);
                break;
            }
        }
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 999 })
            .await;
        assert_no_logout_finished(&mut action_rx);
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
        assert_eq!(probe_rx.recv().await, Some("session_store_closed"));
        assert_eq!(probe_rx.recv().await, Some("session_persistence_deleted"));
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LogoutFinished])
        ) {}
        loop {
            if let CoreEvent::Account(AccountEvent::LoggedOut { request_id, .. }) =
                event_rx.recv().await.expect("logout event")
            {
                assert_eq!(request_id, original);
                break;
            }
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn permanent_close_failures_never_ack_before_a_success_barrier() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false; 16],
            })
            .await;
        let request_id = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        handle
            .send(AccountMessage::RejectProvisionalSession { request_id })
            .await;
        for _ in 0..4 {
            while probe_rx.recv().await != Some("session_store_close_retrying") {}
            assert_no_logout_finished(&mut action_rx);
            handle
                .send(AccountMessage::RetrySessionTeardown { generation: 1 })
                .await;
        }
        assert_no_logout_finished(&mut action_rx);
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn replacement_install_waits_for_provisional_tasks_to_terminate() {
        let first_homeserver = spawn_quarantine_password_server();
        let second_homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
        for homeserver in [first_homeserver, second_homeserver] {
            let request_id = test_request_id();
            assert!(
                handle
                    .send(AccountMessage::Command(AccountCommand::LoginPassword {
                        request_id,
                        request: LoginRequest {
                            homeserver,
                            username: "fixture-user".to_owned(),
                            password: koushi_state::AuthSecret::new("synthetic-password"),
                            device_display_name: Some("Replacement Barrier Test".to_owned()),
                        },
                    }))
                    .await
            );
            loop {
                if matches!(
                    action_rx.recv().await.as_deref(),
                    Some([AppAction::LoginSucceeded { .. }])
                ) {
                    break;
                }
            }
        }
        assert_eq!(probe_rx.try_recv(), Ok("trust_observer_terminated"));
        assert_eq!(probe_rx.try_recv(), Ok("restricted_sync_terminated"));
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn replacement_close_failure_holds_incoming_until_generation_retry_succeeds() {
        let first_homeserver = spawn_quarantine_password_server();
        let second_homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        let first_request = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: first_request,
                request: LoginRequest {
                    homeserver: first_homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginSucceeded { .. }])
        ) {}
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false, true],
            })
            .await;
        let replacement_request = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(2),
            sequence: 2,
        };
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: replacement_request,
                request: LoginRequest {
                    homeserver: second_homeserver.clone(),
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        while probe_rx.recv().await != Some("session_store_close_retrying") {}
        assert_no_login_succeeded_for(&mut action_rx, &second_homeserver);
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (false, false, false, false)
        );

        let later = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(3),
            sequence: 3,
        };
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: later,
                request: LoginRequest {
                    homeserver: "http://127.0.0.1:9".to_owned(),
                    username: "later".to_owned(),
                    password: koushi_state::AuthSecret::new("not-used"),
                    device_display_name: None,
                },
            }))
            .await;
        loop {
            if let CoreEvent::OperationFailed {
                request_id,
                failure,
            } = event_rx.recv().await.expect("later rejection")
                && request_id == later
            {
                assert_eq!(failure, CoreFailure::SessionRequired);
                break;
            }
        }
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 999 })
            .await;
        assert_no_login_succeeded_for(&mut action_rx, &second_homeserver);
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
        while probe_rx.recv().await != Some("replacement_teardown_complete") {}
        loop {
            let actions = action_rx.recv().await.expect("replacement action");
            if matches!(
                actions.as_slice(),
                [AppAction::LoginSucceeded { info, .. }] if info.homeserver == second_homeserver
            ) {
                break;
            }
        }
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn real_store_switch_a_to_b_preserves_both_accounts_and_switches_back() {
        let server_a = spawn_named_quarantine_password_server("@alpha:example.invalid", "DEVICEA");
        let server_b = spawn_named_quarantine_password_server("@beta:example.invalid", "DEVICEB");
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        for (sequence, homeserver) in [(1, server_a.clone()), (2, server_b.clone())] {
            configure_verified_trust(&handle).await;
            let request_id = RequestId {
                connection_id: crate::ids::RuntimeConnectionId(9),
                sequence,
            };
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: None,
                    },
                }))
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        }

        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        let saved = backend.load_saved_sessions().expect("saved index");
        assert_eq!(saved.sessions().len(), 2);
        let alpha_key = saved
            .sessions()
            .iter()
            .find(|key| key.user_id == "@alpha:example.invalid")
            .expect("alpha saved")
            .clone();
        let beta_key = saved
            .sessions()
            .iter()
            .find(|key| key.user_id == "@beta:example.invalid")
            .expect("beta saved")
            .clone();
        assert!(backend.load_matrix_session(&alpha_key).is_ok());
        assert!(backend.load_matrix_session(&beta_key).is_ok());

        for (sequence, user_id) in [(3, "@alpha:example.invalid"), (4, "@beta:example.invalid")] {
            configure_verified_trust(&handle).await;
            handle
                .send(AccountMessage::Command(AccountCommand::SwitchAccount {
                    request_id: RequestId {
                        connection_id: crate::ids::RuntimeConnectionId(9),
                        sequence,
                    },
                    account_key: AccountKey(user_id.to_owned()),
                }))
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
            let saved = backend
                .load_saved_sessions()
                .expect("saved index after switch");
            assert_eq!(saved.sessions().len(), 2);
            assert!(backend.load_matrix_session(&alpha_key).is_ok());
            assert!(backend.load_matrix_session(&beta_key).is_ok());
            assert_eq!(
                backend
                    .load_last_session()
                    .expect("last pointer after switch")
                    .expect("last pointer present")
                    .user_id,
                user_id
            );
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn same_key_replacement_preserves_open_store_and_restores_again_once() {
        let homeserver =
            spawn_named_quarantine_password_server("@same-key:example.invalid", "SAMEDEVICE");
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        for sequence in [1, 2] {
            configure_verified_trust(&handle).await;
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id: RequestId {
                        connection_id: crate::ids::RuntimeConnectionId(11),
                        sequence,
                    },
                    request: LoginRequest {
                        homeserver: homeserver.clone(),
                        username: "same-key".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: None,
                    },
                }))
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        }
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        let saved = backend.load_saved_sessions().expect("saved same-key index");
        assert_eq!(saved.sessions().len(), 1);
        let key_id = saved.sessions()[0].clone();
        assert!(backend.load_matrix_session(&key_id).is_ok());
        assert!(recursive_file_count(data_dir.path()) > 0);

        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::SwitchAccount {
                request_id: RequestId {
                    connection_id: crate::ids::RuntimeConnectionId(11),
                    sequence: 3,
                },
                account_key: AccountKey("@same-key:example.invalid".to_owned()),
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert!(backend.load_matrix_session(&key_id).is_ok());
        assert!(recursive_file_count(data_dir.path()) > 0);
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    async fn inspect_session_runtime(handle: &AccountActorHandle) -> (bool, bool, bool, bool) {
        let (response, result) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::InspectSessionRuntime { response })
                .await
        );
        result.await.expect("runtime inspection")
    }

    async fn inspect_sync_owners(handle: &AccountActorHandle) -> (bool, bool, bool) {
        let (response, result) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::InspectSyncOwners { response })
                .await
        );
        result.await.expect("sync owner inspection")
    }

    async fn shutdown_and_ack(handle: &AccountActorHandle) {
        let (acknowledged, ack) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::ShutdownWithAck { acknowledged })
                .await
        );
        ack.await.expect("account shutdown acknowledgement");
    }

    async fn configure_verified_trust(handle: &AccountActorHandle) {
        let updates = futures_util::stream::pending();
        assert!(
            handle
                .send(AccountMessage::ConfigureTrustObservation {
                    observation: koushi_sdk::CurrentDeviceTrustObservation {
                        current: koushi_state::CurrentDeviceTrustState::Verified,
                        updates: Box::pin(updates),
                    },
                })
                .await
        );
    }

    async fn acknowledge_next_verified_projection(
        handle: &AccountActorHandle,
        action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
    ) {
        let generation = loop {
            let actions = action_rx.recv().await.expect("account actions");
            if let [
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                },
            ] = actions.as_slice()
            {
                break (*generation, *transition_id);
            }
        };
        let (generation, transition_id) = generation;
        assert!(
            handle
                .send(AccountMessage::TrustProjectionApplied {
                    generation,
                    transition_id,
                    ready: true,
                    locked: false,
                })
                .await
        );
        assert_eq!(
            inspect_session_runtime(handle).await,
            (true, true, true, true)
        );
    }

    fn assert_no_logout_finished(action_rx: &mut mpsc::Receiver<Vec<AppAction>>) {
        while let Ok(actions) = action_rx.try_recv() {
            assert!(
                !matches!(actions.as_slice(), [AppAction::LogoutFinished]),
                "teardown acknowledged logout before close barrier"
            );
        }
    }

    fn assert_no_login_succeeded_for(
        action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
        homeserver: &str,
    ) {
        while let Ok(actions) = action_rx.try_recv() {
            assert!(!matches!(
                actions.as_slice(),
                [AppAction::LoginSucceeded { info, .. }] if info.homeserver == homeserver
            ));
        }
    }

    #[tokio::test]
    async fn restore_installs_provisional_without_normal_sync_or_public_ready_event() {
        let homeserver = spawn_quarantine_password_server();
        let login = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("Quarantine Test".to_owned()),
            },
            None,
        )
        .await
        .expect("fixture login");
        let key_id = session_key_id_from_info(&login.info);
        let stored = StoredMatrixSession::new(
            login
                .persistable_session()
                .expect("persistable")
                .to_json()
                .expect("json"),
        );
        drop(login);

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        backend
            .save_matrix_session(&key_id, &stored)
            .expect("session seed");
        backend.remember_saved_session(&key_id).expect("index seed");
        backend.save_last_session(&key_id).expect("pointer seed");

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
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::RestoreSessionSucceeded(_)])
        ));
        let public_ready = executor::timeout(Duration::from_millis(100), async {
            loop {
                match event_rx.recv().await.expect("event stream") {
                    CoreEvent::Account(AccountEvent::SessionRestored { .. })
                    | CoreEvent::Sync(_) => return true,
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            public_ready.is_err(),
            "restore escaped quarantine before Verified"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    fn recursive_file_count(path: &std::path::Path) -> usize {
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    recursive_file_count(&path)
                } else {
                    1
                }
            })
            .sum()
    }

    fn spawn_quarantine_password_server() -> String {
        spawn_named_quarantine_password_server("@fixture-user:example.invalid", "FIXTUREDEVICE")
    }

    fn spawn_controllable_quarantine_password_server()
    -> (String, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let offline = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let homeserver = spawn_named_quarantine_password_server_with_offline(
            "@fixture-user:example.invalid",
            "FIXTUREDEVICE",
            Some(std::sync::Arc::clone(&offline)),
        );
        (homeserver, offline)
    }

    fn spawn_named_quarantine_password_server(
        user_id: &'static str,
        device_id: &'static str,
    ) -> String {
        spawn_named_quarantine_password_server_with_offline(user_id, device_id, None)
    }

    fn spawn_named_quarantine_password_server_with_offline(
        user_id: &'static str,
        device_id: &'static str,
        offline: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            for _ in 0..256 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).expect("read");
                    request.extend_from_slice(&buffer[..count]);
                    let text = String::from_utf8_lossy(&request);
                    let Some(end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let length = text
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                if offline
                    .as_ref()
                    .is_some_and(|offline| offline.load(std::sync::atomic::Ordering::SeqCst))
                {
                    continue;
                }
                let body = if text.starts_with("GET /_matrix/client/versions ") {
                    r#"{"versions":["v1.7"]}"#.to_owned()
                } else if text.contains("/_matrix/client/") && text.contains("login") {
                    format!(
                        r#"{{"access_token":"fixture-token","device_id":"{device_id}","user_id":"{user_id}"}}"#
                    )
                } else if text.contains("/_matrix/client/") && text.contains("/sync") {
                    std::thread::sleep(Duration::from_millis(20));
                    r#"{"next_batch":"batch","device_lists":{"changed":[],"left":[]},"rooms":{"invite":{},"join":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_one_time_keys_count":{}}"#.to_owned()
                } else {
                    r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#.to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).expect("write");
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn restore_trace_covers_startup_restore_boundaries_without_private_ids() {
        let source = include_str!("account.rs");
        let restore_last = source
            .split("async fn handle_restore_last_session")
            .nth(1)
            .expect("handle_restore_last_session should exist")
            .split("/// List saved sessions")
            .next()
            .expect("restore_last_session should precede saved-session listing");
        let restore_account = source
            .split("    async fn restore_account")
            .nth(1)
            .expect("restore_account should exist")
            .split("    async fn handle_login_password")
            .next()
            .expect("restore_account should precede login handler");

        assert!(
            restore_last.contains(
                "trace_account_request(\"restore_last_session\", request_id, \"load_pointer\")"
            ),
            "startup restore must log before reading the last-session pointer"
        );
        assert!(
            restore_last.contains(
                "trace_account_request(\"restore_last_session\", request_id, \"pointer_found\")"
            ),
            "startup restore must log that a pointer exists without printing the account id"
        );
        assert!(
            restore_account.contains(
                "trace_account_request(\"restore_account\", request_id, \"load_session\")"
            ),
            "restore must log before loading the persisted Matrix session blob"
        );
        assert!(
            restore_account.contains(
                "trace_account_request(\"restore_account\", request_id, \"store_restore_ok\")"
            ),
            "restore must log successful SDK store restore before sync starts"
        );
        assert!(restore_account.contains("install_provisional_session"));
        assert!(!restore_account.contains("sync_actor_spawned"));
        assert!(
            source.contains("DiagnosticField::request_id"),
            "restore diagnostics must include request ids for correlation"
        );
        assert!(
            !restore_last.contains("account_name()") && !restore_account.contains("account_name()"),
            "startup restore diagnostics must not print account identifiers"
        );
    }

    #[test]
    fn login_success_is_not_blocked_by_optional_account_hydration() {
        let source = include_str!("account.rs");
        let login_body = source
            .split("    async fn handle_login_password")
            .nth(1)
            .expect("handle_login_password should exist")
            .split("    async fn handle_restore_session")
            .next()
            .expect("login handler should precede restore handler");

        let logged_in = login_body
            .find("AccountEvent::LoggedIn")
            .expect("login handler must emit LoggedIn");
        for optional_fetch in [
            "own_profile_action_from_session(&session_arc).await",
            "local_user_aliases_action_from_session(&session_arc).await",
            "ignored_user_ids_action_from_session(&session_arc).await",
        ] {
            if let Some(pos) = login_body.find(optional_fetch) {
                assert!(
                    pos > logged_in,
                    "{optional_fetch} must not run before LoggedIn; optional account hydration must not block login success"
                );
            }
        }
        assert!(!login_body.contains("spawn_account_hydration"));
        let promotion = source
            .split("async fn handle_current_device_trust")
            .nth(1)
            .and_then(|body| body.split("async fn persist_session").next())
            .expect("trust promotion handler");
        assert!(promotion.contains("spawn_account_hydration"));
    }

    #[test]
    fn stale_projection_ack_does_not_consume_pending_promotion() {
        let pending = PendingTrustTransition {
            generation: 7,
            transition_id: 42,
            decision: TrustLifecycleDecision::Promote,
        };
        assert!(!trust_projection_ack_matches(&pending, 7, 41, false, false));
        assert!(trust_projection_ack_matches(&pending, 7, 42, true, false));
    }

    #[test]
    fn async_account_hydration_is_generation_gated() {
        let source = include_str!("account.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source should precede tests");
        assert!(
            production_source.contains("AccountHydrationLoaded {"),
            "optional account hydration should return through the actor mailbox"
        );
        assert!(
            production_source.contains("generation != self.account_hydration_generation"),
            "stale account hydration from an old session must be dropped before reducer actions are sent"
        );
        assert!(
            production_source.contains("fn invalidate_account_hydration(&mut self)"),
            "session clear/lock must invalidate in-flight account hydration"
        );
    }

    #[test]
    fn session_change_observer_routes_unknown_token_to_session_lock() {
        let source = include_str!("account.rs");
        let observer_body = source
            .split("fn start_session_change_observer")
            .nth(1)
            .and_then(|rest| {
                rest.split("fn next_incoming_verification_request_id")
                    .next()
            })
            .expect("start_session_change_observer body");
        let handler_body = source
            .split("async fn handle_session_invalidated")
            .nth(1)
            .and_then(|rest| rest.split("async fn cancel_identity_reset_handle").next())
            .expect("handle_session_invalidated body");

        assert!(
            observer_body.contains("subscribe_to_session_changes()"),
            "AccountActor must subscribe to the SDK session-change channel; sync errors are not a reliable auth-invalidated source"
        );
        assert!(
            observer_body.contains("matrix_sdk::SessionChange::UnknownToken(data)"),
            "UnknownToken must be handled explicitly instead of inferred from SyncService Offline/Error"
        );
        assert!(
            observer_body.contains("soft_logout: data.soft_logout"),
            "only the private-data-free soft_logout bool may cross into AccountActor"
        );
        assert!(
            handler_body.contains("AppAction::SessionLocked"),
            "auth invalidation must lock the active session so the GUI can offer reauth"
        );
        assert!(
            handler_body.contains("self.stop_sync_actor().await"),
            "auth invalidation must stop the old sync loop instead of leaving it reconnecting forever"
        );
    }

    #[test]
    fn sync_stop_command_must_not_spawn_missing_sync_actor() {
        let source = include_str!("account.rs");
        let route_body = source
            .split("async fn route_sync_command")
            .nth(1)
            .and_then(|rest| rest.split("async fn spawn_sync_actor").next())
            .expect("route_sync_command body");
        let spawn_gate = route_body
            .find("!matches!(command, SyncCommand::Stop { .. })")
            .expect("Stop must be excluded from the missing-actor spawn path");
        let spawn_call = route_body
            .find("self.spawn_sync_actor(session.clone()).await")
            .expect("non-stop sync commands may spawn the actor");
        let no_actor_trace = route_body
            .find("action=no_sync_actor")
            .expect("Stop with an existing session and no actor must be an explicit no-op");

        assert!(
            spawn_gate < spawn_call,
            "route_sync_command must check for Stop before spawning a missing sync actor"
        );
        assert!(
            spawn_call < no_actor_trace,
            "no-actor stop handling should be separate from the spawn path"
        );
    }

    #[test]
    fn restricted_sync_once_guard_precedes_sync_actor_spawn_and_send() {
        let source = include_str!("account.rs");
        let route_body = source
            .split("async fn route_sync_command")
            .nth(1)
            .and_then(|rest| rest.split("async fn spawn_sync_actor").next())
            .expect("route_sync_command body");
        let guard = route_body
            .find("restricted_sync_blocks_sync_once(")
            .expect("restricted sync must gate SyncOnce at the AccountActor boundary");
        let spawn = route_body
            .find("self.spawn_sync_actor(")
            .expect("normal routing should retain lazy SyncActor spawn");
        let send = route_body
            .find("handle.send(SyncMessage::Command(command))")
            .expect("normal routing should retain SyncActor send");

        assert!(guard < spawn && guard < send);
        assert!(
            route_body[guard..spawn].contains("CoreFailure::SyncFailed")
                && route_body[guard..spawn].contains("SyncFailureKind::Internal")
                && route_body[guard..spawn].contains("return;"),
            "restricted-owner rejection must emit one fixed Internal failure before routing"
        );
    }

    #[test]
    fn restricted_sync_blocks_only_sync_once_commands() {
        let request_id = test_request_id();

        assert!(restricted_sync_blocks_sync_once(
            true,
            &SyncCommand::SyncOnce { request_id },
        ));
        assert!(!restricted_sync_blocks_sync_once(
            false,
            &SyncCommand::SyncOnce { request_id },
        ));
        for command in [
            SyncCommand::Start { request_id },
            SyncCommand::Stop { request_id },
            SyncCommand::Restart { request_id },
        ] {
            assert!(!restricted_sync_blocks_sync_once(true, &command));
        }
    }

    #[cfg(feature = "qa-bin")]
    #[test]
    fn qa_device_key_refresh_queries_before_asserting_the_exact_device() {
        let source = include_str!("account.rs");
        let helper = source
            .split("async fn refresh_device_keys_and_assert_known")
            .nth(1)
            .expect("QA device-key refresh helper should exist")
            .split("#[cfg(test)]")
            .next()
            .expect("tests should follow the QA device-key refresh helper");
        let query = helper
            .find("request_user_identity(&user_id)")
            .expect("QA checkpoint must perform an explicit /keys/query");
        let exact_device = helper
            .find("get_device(&user_id, &device_id)")
            .expect("QA checkpoint must require the exact device after refresh");

        assert!(query < exact_device);
        assert!(helper[exact_device..].contains(".ok_or(())?"));
    }

    #[cfg(feature = "qa-bin")]
    #[tokio::test]
    async fn qa_device_key_refresh_accepts_identityless_exact_device_and_rejects_missing_device() {
        let server = MatrixMockServer::new().await;
        server.mock_crypto_endpoints_preset().await;
        let (alice, bob) = server.set_up_alice_and_bob_for_encryption().await;
        let bob_target = VerificationTarget {
            user_id: bob.user_id().expect("synthetic Bob user").to_string(),
            device_id: bob.device_id().expect("synthetic Bob device").to_string(),
        };
        let session = MatrixClientSession::from_client_for_testing(
            alice,
            koushi_state::SessionInfo {
                homeserver: server.uri(),
                user_id: "@alice:example.org".to_owned(),
                device_id: "4L1C3".to_owned(),
            },
        );

        assert_eq!(
            refresh_device_keys_and_assert_known(&session, bob_target.clone()).await,
            Ok(())
        );
        assert_eq!(
            refresh_device_keys_and_assert_known(
                &session,
                VerificationTarget {
                    device_id: "MISSINGDEVICE".to_owned(),
                    ..bob_target
                },
            )
            .await,
            Err(())
        );
    }

    #[test]
    fn session_established_handoff_to_room_actor_is_reliable() {
        let source = include_str!("account.rs");
        let spawn_body = source
            .split("async fn spawn_sync_actor")
            .nth(1)
            .and_then(|rest| rest.split("async fn start_recovery_observer").next())
            .expect("spawn_sync_actor body");
        let session_handoff = spawn_body
            .find(".send(RoomMessage::SessionEstablished")
            .expect("RoomActor session handoff should use reliable send");

        assert!(
            !spawn_body.contains("room_actor.try_send(RoomMessage::SessionEstablished"),
            "SessionEstablished must not be delivered through drop-on-full try_send"
        );
        assert!(
            spawn_body[session_handoff..].contains(".await"),
            "SessionEstablished handoff must await reliable delivery before dependent actors start"
        );
    }

    #[test]
    fn account_actor_reducer_actions_use_reliable_delivery() {
        let source = include_str!("account.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source should precede tests");
        let send_actions_body = production_source
            .split("async fn send_actions")
            .nth(1)
            .and_then(|rest| rest.split("fn trace_room_route").next())
            .expect("AccountActor reliable reducer delivery helper");

        assert!(
            send_actions_body.contains("self.action_tx.send(actions).await"),
            "AccountActor reducer actions must await reliable delivery"
        );
        assert!(
            !production_source.contains("self.reduce("),
            "AccountActor command-result reducer actions must not use the lossy reduce helper"
        );
        assert!(
            !production_source.contains("action_tx.try_send(actions)"),
            "AccountActor reducer actions must not be dropped through try_send"
        );
    }

    #[test]
    fn soft_logout_reauth_keeps_locked_session_until_password_login_succeeds() {
        let source = include_str!("account.rs");
        let handler_body = source
            .split("async fn handle_soft_logout_reauth")
            .nth(1)
            .and_then(|rest| rest.split("fn project_account_management_failure").next())
            .expect("handle_soft_logout_reauth body");
        let login_call = handler_body
            .find("koushi_sdk::login_with_existing_device")
            .expect("reauth must use device-preserving password login");
        let drop_old_session = handler_body
            .find("drop(self.session.take())")
            .expect("reauth must drop the old client before restoring into the account store");

        assert!(
            login_call < drop_old_session,
            "wrong passwords must not discard the locked session before the user can retry"
        );
    }

    #[tokio::test]
    async fn soft_logout_reauth_joins_old_observers_before_subscribing_replacements() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        while probe_rx.try_recv().is_ok() {}

        assert!(
            handle
                .send(AccountMessage::SessionInvalidated { soft_logout: true })
                .await
        );
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SessionLocked])
        ) {}

        let request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::SoftLogoutReauth {
                    request_id,
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                }))
                .await
        );
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([
                AppAction::SoftLogoutReauthSucceeded { request_id: 2 },
                AppAction::LoginSucceeded { .. }
            ])
        ) {}
        let _ = inspect_session_runtime(&handle).await;

        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        let recovery_stop = tokens
            .iter()
            .position(|token| *token == "recovery_observer_stop_requested")
            .expect("the old recovery observer must be stopped");
        let recovery_join = tokens
            .iter()
            .position(|token| *token == "recovery_observer_terminated")
            .expect("the old recovery observer must be joined");
        let recovery_start = tokens
            .iter()
            .position(|token| *token == "recovery_observer_started")
            .expect("the replacement recovery observer must start");
        let verification_stop = tokens
            .iter()
            .position(|token| *token == "incoming_verification_observer_stop_requested")
            .expect("the old verification observer must be stopped");
        let verification_join = tokens
            .iter()
            .position(|token| *token == "incoming_verification_observer_terminated")
            .expect("the old verification observer must be joined");
        let verification_subscribe = tokens
            .iter()
            .position(|token| *token == "incoming_verification_observer_subscribing")
            .expect("the replacement verification observer must subscribe");
        assert!(
            recovery_stop < recovery_join && recovery_join < recovery_start,
            "{tokens:?}"
        );
        assert!(
            verification_stop < verification_join && verification_join < verification_subscribe,
            "{tokens:?}"
        );

        let _ = handle.send(AccountMessage::Shutdown).await;
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

    #[tokio::test]
    async fn recovery_cancel_is_processed_while_task_is_pending_and_stale_result_is_ignored() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let flow_id = 71;
        assert!(
            handle
                .send(AccountMessage::ConfigureSyntheticRecoveryTask {
                    flow_id,
                    pending: true
                })
                .await
        );
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::CancelVerification {
                        request_id: test_request_id(),
                        flow_id,
                        reason: VerificationCancelReason::User,
                    },
                ))
                .await
        );
        let actions = tokio::time::timeout(std::time::Duration::from_secs(1), action_rx.recv())
            .await
            .expect("cancel projection timeout")
            .expect("cancel projection");
        assert_eq!(
            actions,
            vec![AppAction::VerificationGateAttemptFailed {
                kind: koushi_state::VerificationGateFailureKind::Cancelled,
            }]
        );
        let (response, pending) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::InspectRecoveryTask { response })
                .await
        );
        assert!(!pending.await.expect("recovery task inspection"));

        assert!(
            handle
                .send(AccountMessage::RecoveryFinished {
                    generation: 0,
                    flow_id,
                    request_id: incoming_verification_request_id(flow_id),
                    result: Ok(()),
                })
                .await
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), action_rx.recv())
                .await
                .is_err(),
            "stale recovery result must not project a second terminal"
        );
        shutdown_and_ack(&handle).await;
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
            provisional_persistable: None,
            session_promoted: false,
            trust_generation: 0,
            trust_observer: None,
            verification_method_discovery_task: None,
            verification_method_discovery_serial: 0,
            verification_method_discovery_failed: false,
            recovery_task: None,
            restricted_sync: None,
            pending_ready_events: Vec::new(),
            pending_trust_transition: None,
            next_trust_transition_id: 0,
            pending_session_teardown: None,
            next_teardown_generation: 0,
            teardown_retry_task: None,
            lifecycle_probe: None,
            trust_observation_override: std::sync::Mutex::new(None),
            trust_observation_is_synthetic: false,
            recovery_download_override: std::sync::Mutex::new(None),
            close_store_results: std::collections::VecDeque::new(),
            store,
            action_tx,
            event_tx,
            command_rx,
            self_tx,
            sync_actor: None,
            sync_generation: Arc::new(AtomicU64::new(0)),
            room_actor,
            timeline_manager,
            messages_backpressure,
            activity_resolution_task: None,
            data_dir: data_dir_path,
            link_preview_policy: LinkPreviewContext::default(),
            pending_oidc_login: None,
            oidc_completion_override: None,
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            identity_reset_flow_id: None,
            identity_reset_timeout_task: None,
            device_session_ordinals: BTreeMap::new(),
            pending_uia_operations: BTreeMap::new(),
            verification_request: None,
            sas_verification: None,
            own_user_verification: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            sas_timeout_task: None,
            synthetic_verification: None,
            incoming_verification_observer: None,
            incoming_verification_session_generation: 0,
            session_change_observer: None,
            account_hydration_task: None,
            account_hydration_generation: 0,
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
        let invalid_passphrase =
            koushi_sdk::E2eeTrustError::Sdk("invalid passphrase MAC".to_owned());
        assert_eq!(
            classify_e2ee_trust_error(&invalid_passphrase),
            koushi_state::TrustOperationFailureKind::InvalidPassphrase
        );
        assert_eq!(
            classify_e2ee_trust_auth_failure(&invalid_passphrase),
            AuthFailureKind::Sdk
        );
    }

    #[test]
    fn e2ee_key_management_failures_use_typed_classification() {
        let source = include_str!("account.rs");
        let export_handler = source
            .split("async fn handle_export_room_keys")
            .nth(1)
            .expect("export handler should exist")
            .split("async fn handle_import_room_keys")
            .next()
            .expect("import handler should follow export handler");
        let import_handler = source
            .split("async fn handle_import_room_keys")
            .nth(1)
            .expect("import handler should exist")
            .split("async fn handle_bootstrap_secure_backup")
            .next()
            .expect("secure-backup handler should follow import handler");
        let setup_handler = source
            .split("async fn handle_bootstrap_secure_backup")
            .nth(1)
            .expect("secure-backup handler should exist")
            .split("async fn handle_change_secure_backup_passphrase")
            .next()
            .expect("passphrase-change handler should follow setup handler");
        let passphrase_handler = source
            .split("async fn handle_change_secure_backup_passphrase")
            .nth(1)
            .expect("passphrase-change handler should exist")
            .split("async fn handle_reset_identity")
            .next()
            .expect("identity-reset handler should follow passphrase-change handler");

        for handler in [
            export_handler,
            import_handler,
            setup_handler,
            passphrase_handler,
        ] {
            assert!(
                handler.contains("classify_e2ee_trust_error(&error)"),
                "E2EE key-management failures must preserve coarse typed failure kinds"
            );
            assert!(
                !handler.contains("Err(_)"),
                "E2EE key-management handlers must not erase typed errors before classification"
            );
        }
        assert!(
            source.contains("InvalidPassphrase"),
            "trust failure kinds must distinguish invalid room-key/backup passphrases"
        );
    }

    #[test]
    fn local_user_alias_failure_reconciles_authoritative_aliases() {
        let source = include_str!("account.rs");
        let handler = source
            .split("async fn handle_set_local_user_alias")
            .nth(1)
            .expect("local alias handler should exist")
            .split("async fn handle_download_avatar_thumbnail")
            .next()
            .expect("avatar handler should follow local alias handler");

        assert!(
            handler.contains("local_user_aliases_action_from_session(session).await"),
            "failed local-alias saves must reload authoritative aliases so optimistic display mirrors do not drift"
        );
        assert!(
            handler.contains("AppAction::LocalUserAliasUpdateFailed")
                && handler.contains("AppAction::LocalUserAliasesLoaded"),
            "failure reconciliation must emit both the user-visible failure and the full alias projection"
        );
    }

    #[test]
    fn device_list_failures_are_not_reported_as_store_unavailable() {
        let source = include_str!("account.rs");
        let handler = source
            .split("async fn handle_query_devices")
            .nth(1)
            .expect("device query handler should exist")
            .split("async fn handle_load_account_management_capabilities")
            .next()
            .expect("capabilities handler should follow device query");

        assert!(
            handler.contains("classify_e2ee_trust_auth_failure(&error)"),
            "device list failures must classify SDK/network channel errors"
        );
        assert!(
            !handler.contains("CoreFailure::StoreUnavailable"),
            "device list failures must not masquerade as credential-store failures"
        );
    }

    #[test]
    fn identity_reset_auth_wait_has_cancel_and_timeout_exits() {
        let source = include_str!("account.rs");
        let actor_fields = source
            .split("pub struct AccountActor {")
            .nth(1)
            .expect("account actor fields should exist")
            .split("impl AccountActor")
            .next()
            .expect("account actor impl should follow fields");
        let command_route = source
            .split("async fn handle_command")
            .nth(1)
            .expect("command router should exist")
            .split("async fn route_sync_command")
            .next()
            .expect("sync router should follow command router");
        let cancel_handler = source
            .split("async fn handle_cancel_identity_reset")
            .nth(1)
            .expect("identity reset cancel handler should exist")
            .split("async fn handle_submit_identity_reset_auth")
            .next()
            .expect("auth submit handler should follow cancel handler");
        let auth_required_branch = source
            .split("IdentityResetOutcome::AuthRequired(handle)")
            .nth(1)
            .expect("identity reset auth-required branch should exist")
            .split("Err(error)")
            .next()
            .expect("error branch should follow auth-required branch");
        let timeout_handler = source
            .split("async fn handle_identity_reset_auth_timeout")
            .nth(1)
            .expect("identity reset timeout handler should exist")
            .split("async fn handle_submit_identity_reset_auth")
            .next()
            .expect("auth submit handler should follow timeout handler");
        let cleanup = source
            .split("async fn cancel_identity_reset_handle")
            .nth(1)
            .expect("identity reset cleanup helper should exist")
            .split("async fn stop_verification_request_observer")
            .next()
            .expect("verification cleanup should follow identity reset cleanup");

        assert!(
            actor_fields.contains("identity_reset_timeout_task"),
            "AccountActor must retain a timeout task for the pending identity-reset UIA wait"
        );
        assert!(
            command_route.contains("AccountCommand::CancelIdentityReset"),
            "identity reset must expose a user-invocable cancel command"
        );
        assert!(
            cancel_handler.contains("AppAction::ResetIdentityCancelled"),
            "cancel command must settle reducer state through a typed cancel action"
        );
        assert!(
            auth_required_branch.contains("spawn_identity_reset_auth_timeout"),
            "auth-required identity reset waits must schedule a bounded timeout"
        );
        assert!(
            timeout_handler.contains("AppAction::ResetIdentityTimedOut"),
            "timeout must settle reducer state through a typed timeout action"
        );
        assert!(
            cleanup.contains("identity_reset_timeout_task"),
            "identity reset cleanup must abort the timeout task together with the SDK handle"
        );
    }

    #[test]
    fn account_actor_credential_store_hot_paths_use_blocking_port() {
        let source = include_str!("account.rs");
        let persist_session = source
            .split("async fn persist_session")
            .nth(1)
            .expect("persist session helper should be async")
            .split("async fn restore_into_store")
            .next()
            .expect("restore helper should follow persist session");
        let clear_persistence = source
            .split("async fn clear_account_persistence")
            .nth(1)
            .expect("clear persistence helper should be async")
            .split("async fn lookup_session_key_id")
            .next()
            .expect("lookup helper should follow clear persistence");
        let lookup_session = source
            .split("async fn lookup_session_key_id")
            .nth(1)
            .expect("lookup helper should be async")
            .split("fn emit")
            .next()
            .expect("emit helper should follow lookup helper");
        let query_saved = source
            .split("async fn handle_query_saved_sessions")
            .nth(1)
            .expect("saved sessions handler should be async")
            .split("async fn restore_session")
            .next()
            .expect("restore session handler should follow saved sessions");
        let probe_health = source
            .split("async fn handle_probe_local_encryption_health")
            .nth(1)
            .expect("local-encryption probe should exist")
            .split("async fn handle_reset_local_data")
            .next()
            .expect("reset local data should follow probe");

        for section in [
            persist_session,
            clear_persistence,
            lookup_session,
            query_saved,
            probe_health,
        ] {
            assert!(
                section.contains("executor::spawn_blocking"),
                "AccountActor credential-store and filesystem hot paths must be offloaded"
            );
        }
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
