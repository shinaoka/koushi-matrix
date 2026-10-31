//! `actor` ownership for AccountActor.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::{Duration, Instant},
};

#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::SessionKeyId;
use koushi_sdk::{MatrixClientSession, PersistableMatrixSession};
#[cfg(test)]
use koushi_state::DeviceCleanupFailureKind;
use koushi_state::{
    AppAction, AvatarThumbnailState, ComposerDraftRevision, OperationFailureKind,
    SlidingSyncAdmissionSource, SlidingSyncCapabilityResult, SlidingSyncPositiveEvidence,
    TrustOperationFailureKind, VerificationTarget,
};
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};

use crate::composer_draft_lifecycle::{ComposerDraftLeaseRegistry, ForwardedComposerDraftPermit};
#[cfg(test)]
use crate::executor;
use crate::link_preview::LinkPreviewContext;
use crate::native_artifact::{NativeArtifactKind, NativeArtifactPort};
use crate::room::RoomActorHandle;
#[cfg(any(test, feature = "test-hooks"))]
use crate::room::RoomMessage;
#[cfg(any(test, feature = "test-hooks"))]
use crate::room::RoomOperationTestControl;

use crate::search::SearchActorHandle;
use crate::store::StoreActor;
use crate::sync::SyncActorHandle;
use crate::timeline::{
    NavigationProjectionIngress, NavigationProjectionIntent, TimelineManagerHandle,
};
use koushi_protocol::command::{
    AccountCommand, RoomCommand, SearchCommand, SyncCommand, ThreadsListCommand, TimelineCommand,
};
use koushi_protocol::event::{
    CoreEvent, EventCacheFailureReasonClass, EventCacheSubscribeStatus, LocalEncryptionEvent,
};
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::{AccountKey, RequestId, TimelineKey, TimelineKind};

use super::account_management::PendingUiaOperation;
use super::local_data_cleanup::{PendingDeviceCleanup, record_device_cleanup_offer};
use super::profile::AVATAR_DOWNLOAD_CONCURRENCY;
use super::recovery_backup::{
    PendingRecoveryCompletion, PendingRecoveryTask, SECURE_BACKUP_CONNECTIVITY_WAIT_TIMEOUT,
    secure_backup_monitor_wakeup_is_current,
};
#[cfg(test)]
use super::session_lifecycle::PendingOidcFlow;
use super::session_lifecycle::{
    LockedSessionRecord, PendingOidcAttempt, PendingSessionTeardown, SessionChangeObservation,
    SessionInvalidationReason,
};
use super::sliding_sync::{
    PendingSlidingSyncAdmission, PendingSlidingSyncRetry, StoredSlidingSyncAdmissionContext,
};
#[cfg(any(test, feature = "test-hooks"))]
use super::trust_gate::refresh_device_keys_and_assert_known;
use super::trust_gate::{
    OwnedVerificationMethodDiscoveryTask, PendingTrustTransition, RecoveryStateObservation,
    TrustLifecycleDecision, VerificationMethodDiscoveryResult,
    active_own_user_sas_flow_for_provisional_encryption_sync,
    current_device_trust_recheck_failure_token, first_provisional_encryption_sync_is_current,
    method_discovery_admission_timeout_is_current, method_discovery_is_current,
    own_user_sas_recheck_is_current, record_verification_admission_event,
    record_verification_method_discovery_event, retry_should_restart_method_discovery,
    should_discover_verification_methods, verification_admission_event,
    verification_gate_failure_token, verification_method_discovery_event,
};
use super::verification::{
    INCOMING_VERIFICATION_FLOW_ID_BASE, IncomingVerificationObservation, PendingSasVerification,
    PendingVerificationRequest, SasVerificationWaitState, VerificationObservation,
    VerificationTerminal, incoming_verification_request_dropped_event,
    incoming_verification_request_is_current, record_sas_verification_event,
    sas_verification_event,
};
#[cfg(test)]
use super::verification::{SyntheticVerificationTerminal, incoming_verification_request_id};

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
pub(super) use trace_restore;

pub(super) fn trace_account_request(
    stage: &'static str,
    request_id: RequestId,
    action: &'static str,
) {
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

/// Sessions observed at the residency install gap test hook: (old, new).
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) type ResidencyInstallGapSessions = (
    Option<Arc<MatrixClientSession>>,
    Option<Arc<MatrixClientSession>>,
);

/// Maximum number of concurrent direct (non-GUI) searches per account (#1524).
const DIRECT_SEARCH_LIMIT: usize = 4;

/// Maximum number of concurrent direct (non-GUI) message operations per account
/// (#1628).
pub(super) const DIRECT_MESSAGE_ACTION_LIMIT: usize = 4;

/// One absolute deadline for a direct message operation, covering admission,
/// the reaction lane wait, target resolution and submission (#1628).
const DIRECT_MESSAGE_ACTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Messages routed to the AccountActor task.
pub(crate) enum AccountMessage {
    /// A search issued by an out-of-band caller (MCP). Answered on the reply
    /// channel only: no app action and no GUI event is produced.
    /// An attachment fetch for an out-of-band caller (MCP). Answered on the
    /// reply channel only and resolved from the session, so it needs neither a
    /// resident timeline nor a cached media source.
    DirectAttachment {
        room_id: String,
        event_id: String,
        thumbnail: bool,
        response: tokio::sync::oneshot::Sender<
            Result<
                crate::direct_operations::FetchedAttachment,
                crate::direct_operations::AttachmentFetchFailure,
            >,
        >,
    },
    DirectSearch {
        query: String,
        room_ids: Vec<String>,
        content_policy: koushi_state::SearchCrawlerSettings,
        response: tokio::sync::oneshot::Sender<
            Result<
                Vec<koushi_protocol::event::SearchResultItem>,
                crate::search::DirectSearchFailure,
            >,
        >,
    },
    /// A room timeline page for an out-of-band caller (MCP). Answered on the
    /// reply channel only and read from the session, so no timeline actor is
    /// mounted and no GUI state changes.
    DirectSessionAccessToken {
        expected: koushi_state::SessionInfo,
        revision: u64,
        response: tokio::sync::oneshot::Sender<
            Result<
                crate::session_access_token::SessionAccessToken,
                crate::session_access_token::SessionAccessTokenError,
            >,
        >,
    },
    DirectRoomMessages {
        room_id: String,
        limit: u32,
        from: Option<String>,
        response: tokio::sync::oneshot::Sender<
            Result<
                koushi_protocol::event::RoomMessagesPage,
                crate::direct_operations::RoomMessagesFailure,
            >,
        >,
    },
    /// One message read or action for an out-of-band caller (#1628). Answered
    /// on the reply channel only: no app action, no GUI event, no timeline
    /// actor, and no receipt or unread change.
    DirectMessageAction {
        request: crate::direct_operations::DirectMessageRequest,
        response: tokio::sync::oneshot::Sender<
            Result<
                crate::direct_operations::DirectMessageResponse,
                crate::direct_operations::DirectMessageFailure,
            >,
        >,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    PauseHistoryKeyImport {
        disclosure: bool,
        reached: oneshot::Sender<()>,
        resume: oneshot::Receiver<()>,
        installed: oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    HistoryRecoveryBarrier {
        room_id: Option<String>,
        completed: oneshot::Sender<crate::room::HistoryKeyRecoveryDiagnostics>,
    },
    ReadReceiptWindow {
        source: koushi_protocol::view::ReceiptSourceRef,
        start: u64,
        limit: koushi_protocol::view::ReaderWindowLimit,
        response: oneshot::Sender<
            Result<crate::timeline::RawReceiptWindow, crate::view_scope_lifecycle::ScopeError>,
        >,
    },
    Command(AccountCommand),
    /// `AccountCommand::ExportHistory` with the Rust-resolved catalog locale
    /// Element's `export_date` is formatted in.
    ExportHistory {
        request_id: RequestId,
        request: koushi_protocol::command::HistoryExportRequest,
        locale: koushi_state::CatalogLocale,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    QaSetLocalDeviceBlacklisted {
        target: VerificationTarget,
        room_id: String,
        acknowledged: oneshot::Sender<Result<(), ()>>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    QaRefreshDeviceKeysAndAssertKnown {
        target: VerificationTarget,
        acknowledged: oneshot::Sender<Result<(), ()>>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    QaAssertInboundSessionsStartAtZero {
        room_id: String,
        acknowledged: oneshot::Sender<Result<usize, ()>>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    QaSyncOnce {
        request_id: RequestId,
    },
    ContinueSlidingSyncAdmission {
        account_epoch: u64,
        request_id: u64,
        source: SlidingSyncAdmissionSource,
    },
    RetrySlidingSyncCapabilityDiscovery {
        account_epoch: u64,
        blocked_request_id: u64,
        request_id: u64,
    },
    ScheduleSlidingSyncCapabilityRevalidation {
        account_epoch: u64,
    },
    SlidingSyncCapabilityDiscovered {
        account_epoch: u64,
        request_id: u64,
        result: koushi_sdk::SlidingSyncDiscoveryResult,
    },
    SlidingSyncCapabilityRevalidationDiscovered {
        account_epoch: u64,
        request_id: u64,
        result: koushi_sdk::SlidingSyncDiscoveryResult,
    },
    SettleSlidingSyncCapabilityRevalidation {
        account_epoch: u64,
        request_id: u64,
        result: SlidingSyncCapabilityResult,
    },
    SyncCommand(SyncCommand),
    RoomCommand(RoomCommand),
    TimelineCommand(TimelineCommand),
    ReadStatePolicyChanged {
        send_read_receipts: bool,
    },
    DisplayPolicyChanged {
        thread_root_order: koushi_state::TimelineThreadRootOrder,
        /// #1110: applied inside Rust so the renderer never recomputes
        /// visibility from a display preference.
        hide_redacted: bool,
    },
    TimelineCommandWithComposerFormatting {
        command: TimelineCommand,
        formatting_options: koushi_state::ComposerFormattingOptions,
    },
    LeasedTimelineCommandWithComposerFormatting {
        command: TimelineCommand,
        composer_permit: ForwardedComposerDraftPermit,
        formatting_options: koushi_state::ComposerFormattingOptions,
    },
    ResolveActivity {
        generation: u64,
        requests: Vec<crate::activity_resolution::ActivityResolutionRequest>,
    },
    CancelActivityResolution,
    ScheduleServerDelayedSend {
        request_id: RequestId,
        expected_account: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        send_at_ms: u64,
        draft_revision: ComposerDraftRevision,
        composer_permit: ForwardedComposerDraftPermit,
        /// #1348: the composer formatting the schedule was admitted under, so the
        /// stored delayed content matches what the composer would have sent.
        formatting_options: koushi_state::ComposerFormattingOptions,
    },
    DispatchLocalScheduledSend {
        request_id: RequestId,
        origin_session_key: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        /// #1348: the live composer formatting for the local fallback dispatch.
        formatting_options: koushi_state::ComposerFormattingOptions,
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
        /// #1348: the live composer formatting for the replaced delayed content.
        formatting_options: koushi_state::ComposerFormattingOptions,
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
        response_tx: oneshot::Sender<super::RoomEventLookupResult>,
    },
    #[cfg(test)]
    ConfigureEventCacheFetchForTesting {
        fetch: oneshot::Receiver<super::RoomEventLookupResult>,
    },
    #[cfg(test)]
    ConfigureSearchActorForTesting {
        handle: Option<SearchActorHandle>,
        acknowledged: oneshot::Sender<()>,
    },
    /// Publish an own-profile avatar source without an SDK round trip, so the
    /// superseded-entry retirement can be tested (#1396).
    #[cfg(test)]
    NoteOwnAvatarSourceForTesting {
        mxc_uri: Option<String>,
        acknowledged: oneshot::Sender<()>,
    },
    RepairRoomTimeline {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
    },
    SearchCommand(SearchCommand),
    /// Export-time retention sample (#1344): answered by the SearchActor (or
    /// settled as unavailable) without holding the account actor.
    SearchRetentionDiagnostics {
        response: oneshot::Sender<Option<crate::search::AccountRetentionDiagnostics>>,
    },
    /// Export-time resident-set sample (#1394): forwarded to the timeline
    /// manager, which answers from its own handles without awaiting an actor.
    TimelineResidencyDiagnostics {
        response: oneshot::Sender<Option<crate::timeline::TimelineResidencyDiagnostics>>,
    },
    /// A Search or Files request with the account's content policy at submission,
    /// instead of waiting for a crawler notification behind a busy mailbox.
    SearchWithPolicy {
        command: SearchCommand,
        content_policy: koushi_state::SearchCrawlerSettings,
    },
    /// Record `AppEffect::NotifySearchCrawlerRoomsAvailable` as a latest-wins
    /// background crawler notification and try to flush it to SearchActor.
    NotifySearchCrawlerRoomsAvailable {
        room_ids: Vec<String>,
        latest_event_ids: std::collections::BTreeMap<String, String>,
        settings: koushi_state::SearchCrawlerSettings,
    },
    CurrentDeviceTrustChanged {
        generation: u64,
        trust: koushi_state::CurrentDeviceTrustState,
    },
    ActiveSessionAccountManagementUrlResolved {
        generation: u64,
        info: koushi_state::SessionInfo,
        url: Option<String>,
    },
    #[cfg(test)]
    ConfigureAccountManagementDiscovery {
        result: oneshot::Receiver<Option<String>>,
    },
    #[cfg(test)]
    InspectSecureBackupOwners {
        response: oneshot::Sender<SecureBackupOwnersSnapshot>,
    },
    #[cfg(test)]
    ConfigureSecureBackupDeferWait {
        wait: Duration,
    },
    /// Keeps promotion from starting the SDK Secure Backup state observer, so
    /// a test owns every gate-check trigger (#1674).
    #[cfg(test)]
    DisableSecureBackupObserver,
    #[cfg(test)]
    SeedRevealedRecoveryKey {
        revealed: super::recovery_backup::RevealedRecoveryKey,
    },
    CheckCurrentDeviceTrust,
    InspectSecureBackup,
    SyncConnectivityChanged {
        proven: bool,
    },
    /// Fired when the connectivity-wait deadline for a deferred inspection
    /// expires. The serial/generation fencing decides whether it is current.
    SecureBackupDeferredDeadlineExpired {
        generation: u64,
        deadline_serial: u64,
    },
    SecureBackupInspectionFinished {
        generation: u64,
        started_at: Instant,
        result: Result<
            koushi_sdk::MatrixSecureBackupInspection,
            koushi_state::SecureBackupInspectionFailure,
        >,
    },
    RetrySecureBackupInspection {
        generation: u64,
        monitor_serial: u64,
    },
    SecureBackupStateChanged {
        generation: u64,
        state: koushi_sdk::MatrixSecureBackupState,
    },
    RefreshCurrentSessionStatus {
        request_id: u64,
        trigger: koushi_state::SessionStatusRefreshTrigger,
        sync_state: koushi_state::CurrentSessionSyncState,
    },
    /// #1009: reducer `ArmCurrentSessionStatusCheck`; replaces the timer.
    ArmCurrentSessionStatusCheck {
        token: u64,
        due_at_ms: u64,
    },
    CurrentSessionStatusTimerFired {
        token: u64,
    },
    /// #1505: the own-session check timer elapsed.
    OwnSessionCheckDue {
        generation: u64,
    },
    /// #1505: an own-session inventory read settled.
    OwnSessionCheckFinished {
        generation: u64,
        result: Result<koushi_sdk::OwnSessionsInventory, koushi_sdk::OwnSessionsInventoryError>,
    },
    TrustRecheckRetryDue {
        serial: u64,
    },
    #[cfg(test)]
    ConfigureSessionCheckClock {
        base_epoch_ms: u64,
    },
    #[cfg(test)]
    InspectSessionCheckTimer {
        response: oneshot::Sender<Option<u64>>,
    },
    /// Whether a trust-recheck demand has joined the in-flight inspection.
    #[cfg(test)]
    InspectTrustRecheckJoined {
        response: oneshot::Sender<bool>,
    },
    /// Whether any own-identity work (a trust recheck or a current-session
    /// inspection) is in flight or owed.
    #[cfg(test)]
    InspectOwnIdentityWorkPending {
        response: oneshot::Sender<bool>,
    },
    /// The SDK's own current-device trust, which settles asynchronously after
    /// login.
    #[cfg(test)]
    InspectSdkDeviceTrust {
        response: oneshot::Sender<Option<koushi_state::CurrentDeviceTrustState>>,
    },
    CurrentSessionStatusRefreshFinished {
        request_id: u64,
        generation: u64,
        sync_state: koushi_state::CurrentSessionSyncState,
        started_at: Instant,
        result: Result<
            koushi_sdk::MatrixCurrentSessionInspection,
            koushi_state::CurrentSessionStatusFailureKind,
        >,
    },
    CurrentDeviceTrustRecheckFinished {
        generation: u64,
        result: Result<
            (
                koushi_state::CurrentDeviceTrustState,
                koushi_state::VerificationAccountKind,
            ),
            koushi_sdk::CurrentDeviceTrustRecheckError,
        >,
    },
    FirstProvisionalEncryptionSyncFinished {
        generation: u64,
        succeeded: bool,
    },
    VerificationMethodDiscoveryAdmissionTimedOut {
        generation: u64,
        serial: u64,
    },
    ProvisionalEncryptionSyncSucceeded {
        generation: u64,
    },
    ProvisionalEncryptionSyncFailed {
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
    RecoveryTrustSettlementTimedOut {
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
        trust: koushi_state::CurrentDeviceTrustState,
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
    InspectPendingDeviceCleanup {
        response: oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    InspectSyncOwners {
        response: oneshot::Sender<(bool, bool, bool)>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    SetCurrentDeviceTrustForTesting {
        trust: koushi_state::CurrentDeviceTrustState,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestConfigureInstallGap {
        reached: oneshot::Sender<ResidencyInstallGapSessions>,
        release: oneshot::Receiver<()>,
        configured: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestConfigureTeardownGap {
        reached: oneshot::Sender<bool>,
        release: oneshot::Receiver<()>,
        configured: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestInstallSession {
        session: Arc<MatrixClientSession>,
        completed: oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestRoomCommand {
        command: RoomCommand,
        accepted: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestConfigureRoomOperation {
        control: RoomOperationTestControl,
        configured: oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestTimelineSnapshot {
        response: oneshot::Sender<(Vec<String>, Vec<String>)>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestTimelineGateSnapshot {
        response: oneshot::Sender<(bool, usize)>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestShutdown {
        acknowledged: oneshot::Sender<bool>,
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
    ConfigureRecoveryResult {
        completion: oneshot::Receiver<Result<(), koushi_sdk::E2eeRecoveryError>>,
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
    ConfigurePendingOidc {
        start_request_id: RequestId,
        homeserver: String,
    },
    #[cfg(test)]
    ConfigureCloseStoreResults {
        results: Vec<bool>,
    },
    #[cfg(test)]
    ConfigureDeviceCleanupResults {
        results: Vec<Result<koushi_sdk::MatrixDeviceCleanupOutcome, DeviceCleanupFailureKind>>,
    },
    #[cfg(test)]
    StopRoomActorForTesting {
        acknowledged: oneshot::Sender<()>,
    },
    #[cfg(test)]
    ConfigureShutdownGate {
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    },
    ShutdownWithAck {
        acknowledged: oneshot::Sender<bool>,
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
    /// The SDK device/identity store changed while a contact's User info is
    /// open (#1024); `generation` fences a replaced or closed contact.
    ContactSecurityStoreChanged {
        generation: u64,
    },
    ContactSecurityLoadFinished {
        request_id: RequestId,
        generation: u64,
        user_id: String,
        result:
            Result<koushi_state::ContactSecuritySummary, koushi_state::ContactSecurityFailureKind>,
        changes:
            Result<koushi_sdk::ContactSecurityChanges, koushi_state::ContactSecurityFailureKind>,
    },
    ContactUserVerificationRequestFinished {
        request_id: RequestId,
        generation: u64,
        target: VerificationTarget,
        result: Result<koushi_sdk::MatrixVerificationRequestHandle, koushi_sdk::E2eeTrustError>,
    },
    VerificationRequestObserverEnded {
        flow_id: u64,
    },
    SasVerificationObserverEnded {
        flow_id: u64,
    },
    IncomingVerificationRequest {
        /// The session this request was observed for. Admission is fenced by
        /// pointer identity against the actor's current session, so restarting
        /// the same session's observer does not retire an observed request.
        session: Arc<MatrixClientSession>,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    },
    SessionInvalidated {
        reason: SessionInvalidationReason,
    },
    /// The SDK rotated the session's access and refresh tokens. MAS issues
    /// single-use refresh tokens, so the stored copy is invalid from this
    /// moment on and must be replaced.
    SessionTokensRefreshed,
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
        fetch_id: tokio::task::Id,
        thumbnail: AvatarThumbnailState,
    },
    #[cfg(test)]
    AvatarFetchIdForTesting {
        mxc_uri: String,
        response_tx: oneshot::Sender<Option<tokio::task::Id>>,
    },
    /// Internal: optional account-data/profile hydration completed after the
    /// session was already projected as ready. Generation-gated so stale
    /// completions from a previous session are dropped.
    AccountHydrationLoaded {
        generation: u64,
        actions: Vec<AppAction>,
        ignored_user_ids: Option<BTreeSet<String>>,
    },
    /// Fire-and-forget stop used only by actor unit tests; production
    /// shutdown always sends `ShutdownWithAck` from the runtime.
    #[cfg(test)]
    Shutdown,
}

/// cfg(test)-only snapshot of the secure-backup inspection owner state.
#[cfg(test)]
pub(crate) struct SecureBackupOwnersSnapshot {
    pub(super) inspection_pending: bool,
    pub(super) has_inspection_task: bool,
    pub(super) has_defer_deadline: bool,
    pub(super) trust_generation: u64,
    pub(super) defer_serial: u64,
    pub(super) holds_revealed_recovery_key: bool,
}

/// Handle to the AccountActor background task.
#[derive(Clone)]
pub struct AccountActorHandle {
    tx: mpsc::Sender<AccountMessage>,
    avatar_demand_tx:
        tokio::sync::watch::Sender<Option<Arc<crate::view_scope_lifecycle::ChargedAvatarDemand>>>,
    avatar_session_generation: Arc<AtomicU64>,
    navigation_projection: NavigationProjectionIngress,
    /// RoomActor's retained post-commit navigation demand (#1060).
    navigation_enrichment: crate::room::NavigationEnrichmentIngress,
    focused_projection_rx:
        Arc<Mutex<Option<mpsc::UnboundedReceiver<crate::timeline::FocusedProjectionCommitted>>>>,
    #[cfg(any(test, feature = "test-hooks"))]
    residency_room_tx: mpsc::Sender<RoomMessage>,
    #[cfg(any(test, feature = "test-hooks"))]
    residency_room_operation_reached_count: Arc<AtomicUsize>,
    native_artifacts: Arc<dyn NativeArtifactPort>,
    account_work: crate::account_work::AccountWorkScheduler,
}

impl AccountActorHandle {
    pub(crate) fn set_search_crawler_speed(&self, speed: koushi_state::SearchCrawlerSpeed) {
        self.account_work.set_search_crawler_speed(speed);
    }

    pub(crate) fn avatar_demand_context(
        &self,
        account_id: String,
    ) -> koushi_state::AvatarDemandContext {
        koushi_state::AvatarDemandContext {
            account_id,
            session_generation: self
                .avatar_session_generation
                .load(std::sync::atomic::Ordering::Acquire),
        }
    }

    pub(crate) fn publish_avatar_demand_snapshot(
        &self,
        demand: Option<Arc<crate::view_scope_lifecycle::ChargedAvatarDemand>>,
    ) {
        self.avatar_demand_tx.send_if_modified(|current| {
            let unchanged = match (&*current, &demand) {
                (Some(current), Some(next)) => Arc::ptr_eq(current, next),
                (None, None) => true,
                _ => false,
            };
            if unchanged {
                return false;
            }
            *current = demand;
            true
        });
    }

    #[cfg(test)]
    pub(crate) fn publish_avatar_demand(
        &self,
        demand: Option<Arc<koushi_state::AvatarDemandState>>,
    ) {
        // Fixture convenience only; production publishers must supply the
        // reservation owned by their runtime's existing ViewBudget.
        self.publish_avatar_demand_snapshot(demand.map(|demand| {
            Arc::new(
                crate::view_scope_lifecycle::ChargedAvatarDemand::new(
                    (*demand).clone(),
                    &crate::view_budget::ViewBudget::default(),
                )
                .expect("small test demand fits budget"),
            )
        }));
    }

    pub(crate) async fn send(&self, msg: AccountMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Enqueue without waiting for mailbox capacity. AppActor uses this for
    /// dispatches that must never hold its loop (#1060).
    pub(crate) fn try_send(
        &self,
        msg: AccountMessage,
    ) -> Result<(), Box<mpsc::error::TrySendError<AccountMessage>>> {
        self.tx.try_send(msg).map_err(Box::new)
    }

    /// Wait for one mailbox slot. Cancel-safe and independent of this handle,
    /// so AppActor can select on it alongside its other ingress while a
    /// dispatch is deferred.
    pub(crate) fn reserve_owned(
        &self,
    ) -> impl std::future::Future<
        Output = Result<mpsc::OwnedPermit<AccountMessage>, mpsc::error::SendError<()>>,
    > + Send
    + 'static {
        self.tx.clone().reserve_owned()
    }

    pub(crate) fn unregister_native_artifact(
        &self,
        request_id: RequestId,
        kind: NativeArtifactKind,
    ) {
        self.native_artifacts.unregister(request_id, kind);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn configure_residency_install_gap(
        &self,
        reached: oneshot::Sender<ResidencyInstallGapSessions>,
        release: oneshot::Receiver<()>,
    ) -> bool {
        let (configured, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestConfigureInstallGap {
                reached,
                release,
                configured,
            })
            .await
        {
            return false;
        }
        acknowledged.await.is_ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn configure_residency_teardown_gap(
        &self,
        reached: oneshot::Sender<bool>,
        release: oneshot::Receiver<()>,
    ) -> bool {
        let (configured, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestConfigureTeardownGap {
                reached,
                release,
                configured,
            })
            .await
        {
            return false;
        }
        acknowledged.await.is_ok()
    }

    #[cfg(test)]
    pub(crate) async fn note_own_avatar_source_for_testing(&self, mxc_uri: Option<String>) {
        let (acknowledged, done) = oneshot::channel();
        if !self
            .send(AccountMessage::NoteOwnAvatarSourceForTesting {
                mxc_uri,
                acknowledged,
            })
            .await
        {
            return;
        }
        let _ = done.await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn install_residency_test_session(&self, session: Arc<MatrixClientSession>) -> bool {
        let (completed, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestInstallSession { session, completed })
            .await
        {
            return false;
        }
        acknowledged.await.unwrap_or(false)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn residency_test_room_command(&self, command: RoomCommand) -> bool {
        let (accepted, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestRoomCommand { command, accepted })
            .await
        {
            return false;
        }
        acknowledged.await.is_ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn configure_room_operation_test_control(
        &self,
        control: RoomOperationTestControl,
    ) -> bool {
        let (configured, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestConfigureRoomOperation {
                control,
                configured,
            })
            .await
        {
            return false;
        }
        acknowledged.await.unwrap_or(false)
    }

    /// Send a room command directly to the real RoomActor without routing it
    /// through AccountActor. Test-only lifecycle probes use this while the
    /// account actor is blocked at a teardown barrier.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn residency_test_room_command_direct(&self, command: RoomCommand) -> bool {
        self.residency_room_tx
            .send(RoomMessage::Command(command))
            .await
            .is_ok()
    }

    /// Send a room command directly to the real RoomActor while the account
    /// actor is held at the install-gap barrier. This is only needed to probe
    /// the deliberate install→SessionEstablished window.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn residency_test_room_command_at_install_gap(&self, command: RoomCommand) -> bool {
        let (processed, acknowledged) = oneshot::channel();
        if self
            .residency_room_tx
            .send(RoomMessage::TestCommand { command, processed })
            .await
            .is_err()
        {
            return false;
        }
        acknowledged.await.is_ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn residency_test_room_operation_reached_count(&self) -> usize {
        self.residency_room_operation_reached_count
            .load(Ordering::SeqCst)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn residency_test_timeline_snapshot(&self) -> Option<(Vec<String>, Vec<String>)> {
        let (response, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestTimelineSnapshot { response })
            .await
        {
            return None;
        }
        acknowledged.await.ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn residency_test_timeline_gate_snapshot(&self) -> Option<(bool, usize)> {
        let (response, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestTimelineGateSnapshot { response })
            .await
        {
            return None;
        }
        acknowledged.await.ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn shutdown_for_testing(&self) -> bool {
        let (acknowledged, completion) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestShutdown { acknowledged })
            .await
        {
            return false;
        }
        completion.await.unwrap_or(false)
    }

    pub(crate) fn admit_navigation_projection(
        &self,
        intent: NavigationProjectionIntent,
        focused: Option<koushi_protocol::ids::TimelineKey>,
    ) -> bool {
        self.navigation_projection.admit(intent, focused)
    }

    /// Replace the desired main-pane focused timeline without waiting for
    /// any mailbox; TimelineManager retires every other focused actor (#1060).
    /// `owner` is the navigation request owning its projection (#1465).
    pub(crate) fn admit_focused_foreground(
        &self,
        focused: Option<koushi_protocol::ids::TimelineKey>,
        owner: Option<koushi_protocol::ids::RequestId>,
    ) {
        self.navigation_projection
            .admit_focused_with_owner(focused, owner);
    }

    /// Hand committed navigation to RoomActor-owned enrichment without
    /// waiting for, or filling, any actor mailbox (#1060).
    pub(crate) fn admit_navigation_enrichment(
        &self,
        session_key: SessionKeyId,
        active_room_id: Option<String>,
        active_space_id: Option<String>,
        space_selected: bool,
    ) {
        self.navigation_enrichment.admit(
            session_key,
            active_room_id,
            active_space_id,
            space_selected,
        );
    }

    #[cfg(test)]
    pub(crate) fn latest_navigation_enrichment(
        &self,
    ) -> Option<crate::room::NavigationEnrichmentDemand> {
        self.navigation_enrichment.latest()
    }

    pub(crate) fn take_focused_projection_commits(
        &self,
    ) -> Option<mpsc::UnboundedReceiver<crate::timeline::FocusedProjectionCommitted>> {
        self.focused_projection_rx
            .lock()
            .expect("focused projection receiver lock must not be poisoned")
            .take()
    }

    #[cfg(test)]
    pub(crate) fn for_app_actor_test(
        tx: mpsc::Sender<AccountMessage>,
        navigation_projection: NavigationProjectionIngress,
    ) -> Self {
        Self {
            tx,
            navigation_projection,
            navigation_enrichment: crate::room::NavigationEnrichmentIngress::channel().0,
            avatar_demand_tx: tokio::sync::watch::channel(None).0,
            avatar_session_generation: Arc::new(AtomicU64::new(0)),
            focused_projection_rx: Arc::new(Mutex::new(None)),
            native_artifacts: Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
            account_work: crate::account_work::AccountWorkScheduler::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            residency_room_tx: {
                let (room_tx, _room_rx) = mpsc::channel(1);
                room_tx
            },
            #[cfg(any(test, feature = "test-hooks"))]
            residency_room_operation_reached_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// The account actor's internal state.
pub struct AccountActor {
    /// Active store-backed session, if any.
    pub(super) session: Option<Arc<MatrixClientSession>>,
    /// Session key for credential store operations.
    pub(super) session_key_id: Option<SessionKeyId>,
    pub(super) locked_session_record: Option<LockedSessionRecord>,
    pub(super) composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    pub(super) provisional_persistable: Option<PersistableMatrixSession>,
    pub(super) sliding_sync_positive_evidence: Option<SlidingSyncPositiveEvidence>,
    pub(super) sliding_sync_account_epoch: u64,
    pub(super) sliding_sync_request_id: u64,
    pub(super) pending_sliding_sync_admission: Option<PendingSlidingSyncAdmission>,
    pub(super) pending_sliding_sync_retry: Option<PendingSlidingSyncRetry>,
    pub(super) stored_sliding_sync_admission: Option<StoredSlidingSyncAdmissionContext>,
    pub(super) sliding_sync_discovery_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) sliding_sync_revalidation_pending: Option<u64>,
    pub(super) sliding_sync_revalidation_request: Option<(u64, u64)>,
    pub(super) sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    pub(super) native_artifacts: Arc<dyn NativeArtifactPort>,
    /// Bounds concurrent direct (non-GUI) searches; their tasks are aborted on
    /// shutdown so no operation outlives the account (#1524).
    pub(super) direct_search_slots: Arc<tokio::sync::Semaphore>,
    pub(super) direct_search_tasks: Vec<crate::executor::JoinHandle<()>>,
    /// Bounded reads of a room's recent timeline for out-of-band callers (#1530).
    pub(super) direct_room_messages_tasks: Vec<crate::executor::JoinHandle<()>>,
    /// Bounds concurrent direct (non-GUI) message operations, and serializes
    /// reaction decisions so two calls cannot both add the same reaction
    /// (#1628).
    pub(super) direct_message_action_slots: Arc<tokio::sync::Semaphore>,
    pub(super) direct_message_action_tasks: Vec<crate::executor::JoinHandle<()>>,
    /// The retry ledger for direct message actions (#1681, #1660).
    pub(super) direct_action_ledger:
        Arc<std::sync::Mutex<super::direct_action_ledger::DirectActionLedger>>,
    pub(super) direct_reaction_lane: Arc<tokio::sync::Mutex<()>>,
    pub(super) direct_attachment_tasks: Vec<crate::executor::JoinHandle<()>>,
    pub(super) session_promoted: bool,
    pub(super) trust_generation: u64,
    pub(super) trust_observer: Option<crate::executor::JoinHandle<()>>,
    pub(super) trust_recheck_task: Option<crate::executor::JoinHandle<()>>,
    /// One losslessly coalesced reducer demand that arrived while an
    /// authoritative query or its projection acknowledgement was unsettled.
    pub(super) trust_recheck_pending: bool,
    pub(super) current_session_status_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) current_session_status_request: Option<u64>,
    pub(super) session_check: super::session_check::SessionCheckCoordinator,
    pub(super) secure_backup_ready: bool,
    pub(super) recovery_key_delivery_pending: bool,
    /// The recovery key currently revealed on screen (#927), held only until
    /// the explicit saved confirmation, session teardown, or account switch.
    pub(super) revealed_recovery_key: Option<super::recovery_backup::RevealedRecoveryKey>,
    pub(super) secure_backup_inspection_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) secure_backup_monitor_task: Option<crate::executor::JoinHandle<()>>,
    /// One-shot deadline task owned while an inspection is deferred waiting
    /// for proven sync connectivity (#860): no deferred wait may be open-ended.
    pub(super) secure_backup_defer_deadline_task: Option<crate::executor::JoinHandle<()>>,
    /// Fences deferred-deadline wakeups; bumped on disarm and on consumption
    /// so stale or replayed expiries cannot re-fail the gate.
    pub(super) secure_backup_defer_serial: u64,
    pub(super) secure_backup_connectivity_wait: Duration,
    pub(super) secure_backup_monitor_serial: u64,
    pub(super) secure_backup_inspection_pending: bool,
    pub(super) sync_connectivity_proven: bool,
    pub(super) secure_backup_retry_attempt: u32,
    pub(super) secure_backup_recovery_epoch: bool,
    pub(super) secure_backup_recovery_reset_consumed: bool,
    pub(super) secure_backup_observer: Option<crate::executor::JoinHandle<()>>,
    pub(super) verification_method_discovery_task: Option<OwnedVerificationMethodDiscoveryTask>,
    pub(super) verification_method_discovery_admission_task:
        Option<crate::executor::JoinHandle<()>>,
    pub(super) verification_method_discovery_serial: u64,
    pub(super) verification_method_discovery_failed: bool,
    pub(super) recovery_task: Option<PendingRecoveryTask>,
    pub(super) pending_recovery_completion: Option<PendingRecoveryCompletion>,
    pub(super) recovery_trust_settlement_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) provisional_encryption_sync: Option<crate::executor::JoinHandle<()>>,
    pub(super) provisional_encryption_sync_ready: bool,
    pub(super) encryption_sync_permit: koushi_sdk::EncryptionSyncPermitOwner,
    pub(super) pending_ready_events: Vec<CoreEvent>,
    pub(super) pending_trust_transition: Option<PendingTrustTransition>,
    pub(super) next_trust_transition_id: u64,
    pub(super) pending_session_teardown: Option<PendingSessionTeardown>,
    pub(super) next_teardown_generation: u64,
    pub(super) teardown_retry_task: Option<crate::executor::JoinHandle<()>>,
    #[cfg(test)]
    pub(super) lifecycle_probe: Option<mpsc::UnboundedSender<&'static str>>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) residency_install_gap: Option<(
        oneshot::Sender<ResidencyInstallGapSessions>,
        oneshot::Receiver<()>,
    )>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) residency_teardown_gap: Option<(oneshot::Sender<bool>, oneshot::Receiver<()>)>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) residency_preserve_room_session: bool,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) trust_observation_override:
        std::sync::Mutex<Option<koushi_sdk::CurrentDeviceTrustObservation>>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) trust_observation_is_synthetic: bool,
    #[cfg(test)]
    pub(super) recovery_download_override: std::sync::Mutex<Option<oneshot::Receiver<bool>>>,
    #[cfg(test)]
    pub(super) recovery_result_override:
        std::sync::Mutex<Option<oneshot::Receiver<Result<(), koushi_sdk::E2eeRecoveryError>>>>,
    #[cfg(test)]
    pub(super) close_store_results: std::collections::VecDeque<bool>,
    #[cfg(test)]
    pub(super) account_management_discovery_override:
        std::sync::Mutex<Option<oneshot::Receiver<Option<String>>>>,
    #[cfg(test)]
    pub(super) device_cleanup_results: std::collections::VecDeque<
        Result<koushi_sdk::MatrixDeviceCleanupOutcome, DeviceCleanupFailureKind>,
    >,
    /// Store actor — owns the credential store backend and per-account paths.
    pub(super) store: StoreActor,
    /// App-level action channel to drive the reducer.
    pub(super) action_tx: mpsc::Sender<Vec<AppAction>>,
    /// Shared event broadcast channel.
    pub(super) event_tx: broadcast::Sender<CoreEvent>,
    /// Message inbox.
    pub(super) command_rx: mpsc::Receiver<AccountMessage>,
    /// Sender clone used by SDK observation tasks to report actor-owned
    /// verification progress back into this actor's mailbox.
    pub(super) self_tx: mpsc::Sender<AccountMessage>,
    /// SyncActor child handle (Phase 3). Present only when a store-backed
    /// session exists. Created on first login/restore; destroyed on logout /
    /// account switch.
    pub(super) sync_actor: Option<SyncActorHandle>,
    /// Monotonic across SyncActor replacement so lifecycle projections from a
    /// restarted actor cannot be rejected behind the previous actor's fence.
    pub(super) sync_generation: Arc<AtomicU64>,
    /// RoomActor child handle (Phase 4). Spawned once at actor creation and
    /// kept alive for the lifetime of the AccountActor. Session is provided
    /// via `RoomMessage::SyncStarted` when sync begins.
    pub(super) room_actor: RoomActorHandle,
    /// TimelineManagerActor handle (Phase 5). Spawned once at actor creation;
    /// session reference is updated when a store-backed session is established.
    pub(super) timeline_manager: TimelineManagerHandle,
    pub(super) read_persistence_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) read_persistence_session_generation: u64,
    /// Stable projection ingress retained across session-scoped manager
    /// replacement and cloned into the AppActor-facing handle.
    pub(super) navigation_projection: NavigationProjectionIngress,
    pub(super) focused_projection_tx:
        mpsc::UnboundedSender<crate::timeline::FocusedProjectionCommitted>,
    /// Account-wide gate for `/rooms/{roomId}/messages` requests. Timeline
    /// pagination has priority over background search-history crawling.
    pub(super) account_work: crate::account_work::AccountWorkScheduler,
    pub(super) activity_resolution_task: Option<crate::executor::JoinHandle<()>>,
    /// The room-history export task, retained until cancel, teardown, or
    /// replacement by the next export.
    pub(super) history_export: Option<super::history_export::ActiveHistoryExport>,
    pub(super) last_history_export: Option<super::history_export::LastHistoryExport>,
    pub(super) history_export_fs: Arc<dyn crate::room_history_export::HistoryExportFilesystem>,
    /// Application data directory for cached preview images.
    pub(super) data_dir: std::path::PathBuf,
    /// Latest link-preview policy snapshot from AppState, kept current so a
    /// newly-created session-scoped timeline manager starts with the right policy.
    pub(super) link_preview_policy: LinkPreviewContext,
    /// Latest Rust-owned receipt policy, replayed into each replacement manager.
    pub(super) send_read_receipts: bool,
    /// SearchActor handle (Phase 6). Present only when a store-backed session
    /// exists. Created at the same time as SyncActor; stopped in the ordered
    /// shutdown between timelines and sync (canon Async rule 12 step 3).
    pub(super) search_actor: Option<SearchActorHandle>,
    /// ThreadsListActor handle. Present only while the threads list view is
    /// open. Dropping the handle cancels the actor and its SDK subscriptions.
    pub(super) threads_list_actor: Option<crate::threads_list::ThreadsListActorHandle>,
    /// Recovery-state observer task for the active store-backed session.
    pub(super) recovery_observer: Option<RecoveryStateObservation>,
    /// Pending SDK identity reset continuation, held only inside AccountActor.
    pub(super) identity_reset_handle: Option<koushi_sdk::MatrixIdentityResetHandle>,
    /// Flow id for the pending SDK identity reset continuation.
    pub(super) identity_reset_flow_id: Option<u64>,
    /// Timeout task for the pending identity reset auth challenge.
    pub(super) identity_reset_timeout_task: Option<crate::executor::JoinHandle<()>>,
    /// Pending UIA operations keyed by the flow id (original request id).
    /// Holds the data needed to retry a destructive action after the user
    /// supplies interactive auth. Secrets (password, UIA session) are held
    /// only inside this actor-private map, never in reducer state.
    pub(super) pending_uia_operations: BTreeMap<u64, PendingUiaOperation>,
    /// Pending notification-email verification (#981): client secret,
    /// validation session id and UIA continuation stay actor-private.
    pub(super) pending_notification_email:
        Option<super::account_notifications::PendingNotificationEmailVerification>,
    /// Last authoritative keyword-rule projection (#1427). Actor-private so a
    /// duplicate, an over-cap add, or an unknown removal is refused before any
    /// request; it is replaced by every authoritative notification read and is
    /// never edited by an optimistic write.
    pub(super) notification_keywords: Vec<koushi_state::NotificationKeywordRuleState>,
    /// Opaque legacy UIAA continuation or local retry context. Raw SDK data
    /// never enters reducer state or diagnostics.
    pub(super) pending_device_cleanup: Option<PendingDeviceCleanup>,
    /// Pending OAuth authorization-code flow, keyed by originating request id.
    /// Holds SDK client, PKCE verifier, and CSRF validation data inside Rust.
    pub(super) pending_oidc_login: Option<PendingOidcAttempt>,
    #[cfg(test)]
    pub(super) oidc_completion_override: Option<MatrixClientSession>,
    #[cfg(test)]
    pub(super) secure_backup_observer_disabled: bool,
    /// Pending SDK verification request continuation, held only inside
    /// AccountActor and never projected into AppState.
    pub(super) verification_request: Option<PendingVerificationRequest>,
    /// Pending SDK SAS continuation, held only inside AccountActor and never
    /// projected into AppState.
    pub(super) sas_verification: Option<PendingSasVerification>,
    pub(super) own_user_verification: Option<(u64, koushi_sdk::MatrixOwnUserVerificationHandle)>,
    pub(super) sas_waiting_for: Option<(u64, SasVerificationWaitState)>,
    /// SDK verification request observer task for the active flow.
    pub(super) verification_request_observer: Option<VerificationObservation>,
    /// SDK SAS observer task for the active flow.
    pub(super) sas_verification_observer: Option<VerificationObservation>,
    pub(super) sas_timeout_task: Option<crate::executor::JoinHandle<()>>,
    #[cfg(test)]
    pub(super) synthetic_verification: Option<(u64, VerificationTarget)>,
    /// SDK incoming verification request observer for the active session.
    pub(super) incoming_verification_observer: Option<IncomingVerificationObservation>,
    /// Open contact's security-details observer (#1024).
    pub(super) contact_security: Option<super::contact_security::ContactSecurityObservation>,
    pub(super) contact_security_generation: u64,
    /// #1505: the own-session check schedule.
    pub(super) own_session_check: super::own_sessions::OwnSessionCheck,
    pub(super) contact_security_load_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) pending_contact_verification_send:
        Option<super::contact_security::PendingContactVerificationSend>,
    pub(super) contact_verification_send_generation: u64,
    /// SDK session-change observer for auth invalidation / soft logout.
    pub(super) session_change_observer: Option<SessionChangeObservation>,
    /// Optional profile/account-data hydration task for the active session.
    pub(super) account_hydration_task: Option<crate::executor::JoinHandle<()>>,
    /// Active-session account-management discovery. Generation-fenced and
    /// aborted whenever the promoted session is no longer usable.
    pub(super) account_management_discovery_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) account_management_discovery_generation: u64,
    /// Incremented whenever optional account hydration is invalidated.
    pub(super) account_hydration_generation: u64,
    /// Synthetic flow id sequence for SDK-originated verification requests.
    pub(super) next_incoming_verification_sequence: u64,
    /// Last `NotifySearchCrawlerRoomsAvailable` payload received before the
    /// `SearchActor` was spawned.  Replayed into the actor immediately after
    /// it is created so rooms that were already known to the reducer at
    /// session-restore time are not missed by the auto-start logic.
    pub(super) pending_crawler_notification: Option<crate::search::CrawlerRoomsNotification>,
    /// Actor-owned avatar thumbnail cache: mxc_uri -> last resolved state.
    /// Mutated only from the actor loop; no shared lock needed.
    pub(super) avatar_cache: HashMap<String, AvatarThumbnailState>,
    #[cfg(test)]
    pub(super) event_cache_fetch_override: Option<oneshot::Receiver<super::RoomEventLookupResult>>,
    /// In-flight fetches: mxc_uri -> waiting request_ids (single-flight dedup).
    /// Entries cover both active and queued distinct resources, so this map is
    /// the bounded logical-demand ledger rather than a task count.
    pub(super) avatar_inflight: HashMap<String, Vec<RequestId>>,
    /// Distinct resources waiting for one of the six active fetch slots. No
    /// task is spawned until a slot is available; the queue is capped by the
    /// public scoped-publication contract.
    pub(super) avatar_pending: VecDeque<String>,
    /// Number of fetch tasks that have acquired an actor-owned active slot.
    pub(super) avatar_active_fetches: usize,
    /// Semaphore bounding concurrent avatar downloads. Cloned into spawned
    /// fetch tasks; the actor holds one Arc so it can be replaced on session
    /// clear.
    pub(super) avatar_download_semaphore: Arc<Semaphore>,
    /// Owns all spawned avatar-fetch tasks. Aborted on session clear and
    /// shutdown (engineering-rules: every spawned task has an owner).
    pub(super) avatar_fetch_tasks: tokio::task::JoinSet<()>,
    /// Abort handles let renderer teardown cancel queued semaphore waiters as
    /// well as network-active avatar fetches.
    pub(super) avatar_fetch_abort_handles: HashMap<String, tokio::task::AbortHandle>,
    /// Incremented by `abort_avatar_fetch_tasks` on every session clear /
    /// logout / switch / shutdown so that `AvatarFetched` completions that
    /// were already enqueued before the abort are detected and silently dropped
    /// instead of being accepted into the new (or absent) session's state.
    pub(super) avatar_session_generation: Arc<AtomicU64>,
    pub(super) avatar_demand_rx:
        tokio::sync::watch::Receiver<Option<Arc<crate::view_scope_lifecycle::ChargedAvatarDemand>>>,
    pub(super) avatar_demand: Option<Arc<crate::view_scope_lifecycle::ChargedAvatarDemand>>,
    /// The own profile's current avatar source, so a change retires the
    /// superseded bytes (#1396).
    pub(super) own_avatar_mxc_uri: Option<String>,
    /// #1433: sources whose bytes another source's store evicted while the
    /// scoped demand still referenced them. A working set larger than the
    /// renderable cache cannot be satisfied by refetching, so these are
    /// terminal for the current demand and retried once when the demanded
    /// resource set changes.
    pub(super) avatar_capacity_evicted: std::collections::HashSet<String>,
}

impl AccountActor {
    pub fn spawn(
        store_actor: StoreActor,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        initial_link_preview_policy: LinkPreviewContext,
        composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    ) -> AccountActorHandle {
        Self::spawn_with_diagnostics(
            store_actor,
            action_tx,
            event_tx,
            initial_link_preview_policy,
            composer_draft_leases,
            true,
            crate::SlidingSyncDiagnostics::default(),
        )
    }

    pub(crate) fn spawn_with_diagnostics(
        store_actor: StoreActor,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        initial_link_preview_policy: LinkPreviewContext,
        composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
        initial_send_read_receipts: bool,
        sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    ) -> AccountActorHandle {
        Self::spawn_with_diagnostics_and_native_artifacts(
            store_actor,
            action_tx,
            event_tx,
            initial_link_preview_policy,
            composer_draft_leases,
            initial_send_read_receipts,
            sliding_sync_diagnostics,
            Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
            crate::account_work::AccountWorkScheduler::default(),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "actor wiring: independent owned handles moved into one task"
    )]
    pub(crate) fn spawn_with_diagnostics_and_native_artifacts(
        store_actor: StoreActor,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        initial_link_preview_policy: LinkPreviewContext,
        composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
        initial_send_read_receipts: bool,
        sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
        native_artifacts: Arc<dyn NativeArtifactPort>,
        account_work: crate::account_work::AccountWorkScheduler,
    ) -> AccountActorHandle {
        // AppActor forwards every Room/Timeline/Sync command here via send().await;
        // sized so heavy sync does not block the AppActor's forwarding.
        let (tx, command_rx) = mpsc::channel(crate::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let (avatar_demand_tx, avatar_demand_rx) = tokio::sync::watch::channel(None);
        let avatar_session_generation = Arc::new(AtomicU64::new(0));
        let data_dir = store_actor.data_dir().to_path_buf();
        // Spawn RoomActor once at AccountActor creation. It starts with no
        // session and waits for RoomMessage::SyncStarted.
        let room_actor = crate::room::RoomActorHandle::spawn_with_account_work(
            action_tx.clone(),
            event_tx.clone(),
            sliding_sync_diagnostics.clone(),
            account_work.clone(),
        );
        let navigation_enrichment = room_actor.navigation_enrichment();
        let (navigation_projection, navigation_projection_rx) =
            NavigationProjectionIngress::channel();
        let (focused_projection_tx, focused_projection_rx) = mpsc::unbounded_channel();
        // Spawn TimelineManagerActor. It starts with no session; the session
        // is injected when a store-backed session is established.
        let timeline_manager = crate::timeline::TimelineManagerHandle::spawn(
            action_tx.clone(),
            event_tx.clone(),
            Some(data_dir.clone()),
            account_work.clone(),
            Some(navigation_projection_rx),
            Some(focused_projection_tx.clone()),
        );
        #[cfg(any(test, feature = "test-hooks"))]
        let residency_room_tx = room_actor.sender();
        #[cfg(any(test, feature = "test-hooks"))]
        let residency_room_operation_reached_count = room_actor.operation_test_reached_count();
        let handle_account_work = account_work.clone();
        let actor = AccountActor {
            session: None,
            session_key_id: None,
            locked_session_record: None,
            composer_draft_leases,
            provisional_persistable: None,
            sliding_sync_positive_evidence: None,
            sliding_sync_account_epoch: 0,
            sliding_sync_request_id: 0,
            pending_sliding_sync_admission: None,
            pending_sliding_sync_retry: None,
            stored_sliding_sync_admission: None,
            sliding_sync_discovery_task: None,
            sliding_sync_revalidation_pending: None,
            sliding_sync_revalidation_request: None,
            sliding_sync_diagnostics,
            native_artifacts: Arc::clone(&native_artifacts),
            direct_search_slots: Arc::new(tokio::sync::Semaphore::new(DIRECT_SEARCH_LIMIT)),
            direct_search_tasks: Vec::new(),
            direct_room_messages_tasks: Vec::new(),
            direct_message_action_slots: Arc::new(tokio::sync::Semaphore::new(
                DIRECT_MESSAGE_ACTION_LIMIT,
            )),
            direct_message_action_tasks: Vec::new(),
            direct_action_ledger: Arc::new(std::sync::Mutex::new(
                super::direct_action_ledger::DirectActionLedger::new(DIRECT_MESSAGE_ACTION_TIMEOUT),
            )),
            direct_reaction_lane: Arc::new(tokio::sync::Mutex::new(())),
            direct_attachment_tasks: Vec::new(),
            session_promoted: false,
            trust_generation: 0,
            trust_observer: None,
            trust_recheck_task: None,
            trust_recheck_pending: false,
            current_session_status_task: None,
            current_session_status_request: None,
            session_check: super::session_check::SessionCheckCoordinator::default(),
            secure_backup_ready: false,
            recovery_key_delivery_pending: false,
            revealed_recovery_key: None,
            secure_backup_inspection_task: None,
            secure_backup_monitor_task: None,
            secure_backup_defer_deadline_task: None,
            secure_backup_defer_serial: 0,
            secure_backup_connectivity_wait: SECURE_BACKUP_CONNECTIVITY_WAIT_TIMEOUT,
            secure_backup_monitor_serial: 0,
            secure_backup_inspection_pending: false,
            sync_connectivity_proven: false,
            secure_backup_retry_attempt: 0,
            secure_backup_recovery_epoch: false,
            secure_backup_recovery_reset_consumed: false,
            secure_backup_observer: None,
            verification_method_discovery_task: None,
            verification_method_discovery_admission_task: None,
            verification_method_discovery_serial: 0,
            verification_method_discovery_failed: false,
            recovery_task: None,
            pending_recovery_completion: None,
            recovery_trust_settlement_task: None,
            provisional_encryption_sync: None,
            provisional_encryption_sync_ready: false,
            encryption_sync_permit: koushi_sdk::new_encryption_sync_permit_owner(),
            pending_ready_events: Vec::new(),
            pending_trust_transition: None,
            next_trust_transition_id: 0,
            pending_session_teardown: None,
            next_teardown_generation: 0,
            teardown_retry_task: None,
            #[cfg(test)]
            lifecycle_probe: None,
            #[cfg(any(test, feature = "test-hooks"))]
            residency_install_gap: None,
            #[cfg(any(test, feature = "test-hooks"))]
            residency_teardown_gap: None,
            #[cfg(any(test, feature = "test-hooks"))]
            residency_preserve_room_session: false,
            #[cfg(any(test, feature = "test-hooks"))]
            trust_observation_override: std::sync::Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            trust_observation_is_synthetic: false,
            #[cfg(test)]
            recovery_download_override: std::sync::Mutex::new(None),
            #[cfg(test)]
            recovery_result_override: std::sync::Mutex::new(None),
            #[cfg(test)]
            close_store_results: std::collections::VecDeque::new(),
            #[cfg(test)]
            account_management_discovery_override: std::sync::Mutex::new(None),
            #[cfg(test)]
            device_cleanup_results: std::collections::VecDeque::new(),
            store: store_actor,
            action_tx,
            event_tx,
            command_rx,
            self_tx: tx.clone(),
            sync_actor: None,
            sync_generation: Arc::new(AtomicU64::new(0)),
            room_actor,
            timeline_manager,
            read_persistence_task: None,
            read_persistence_session_generation: 0,
            navigation_projection: navigation_projection.clone(),
            focused_projection_tx,
            account_work,
            activity_resolution_task: None,
            history_export: None,
            last_history_export: None,
            history_export_fs: super::history_export::default_history_export_fs(),
            data_dir,
            link_preview_policy: initial_link_preview_policy,
            send_read_receipts: initial_send_read_receipts,
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            identity_reset_flow_id: None,
            identity_reset_timeout_task: None,
            pending_uia_operations: BTreeMap::new(),
            pending_notification_email: None,
            notification_keywords: Vec::new(),
            pending_device_cleanup: None,
            pending_oidc_login: None,
            #[cfg(test)]
            oidc_completion_override: None,
            #[cfg(test)]
            secure_backup_observer_disabled: false,
            verification_request: None,
            sas_verification: None,
            own_user_verification: None,
            sas_waiting_for: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            sas_timeout_task: None,
            #[cfg(test)]
            synthetic_verification: None,
            incoming_verification_observer: None,
            contact_security: None,
            contact_security_generation: 0,
            own_session_check: Default::default(),
            contact_security_load_task: None,
            pending_contact_verification_send: None,
            contact_verification_send_generation: 0,
            session_change_observer: None,
            account_hydration_task: None,
            account_management_discovery_task: None,
            account_management_discovery_generation: 0,
            account_hydration_generation: 0,
            next_incoming_verification_sequence: INCOMING_VERIFICATION_FLOW_ID_BASE,
            pending_crawler_notification: None,
            avatar_cache: HashMap::new(),
            #[cfg(test)]
            event_cache_fetch_override: None,
            avatar_inflight: HashMap::new(),
            avatar_pending: VecDeque::new(),
            avatar_active_fetches: 0,
            avatar_download_semaphore: Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY)),
            avatar_fetch_tasks: tokio::task::JoinSet::new(),
            avatar_fetch_abort_handles: HashMap::new(),
            avatar_session_generation: avatar_session_generation.clone(),
            avatar_demand_rx,
            avatar_demand: None,
            own_avatar_mxc_uri: None,
            avatar_capacity_evicted: std::collections::HashSet::new(),
        };
        crate::executor::spawn(actor.run());
        AccountActorHandle {
            tx,
            avatar_demand_tx,
            avatar_session_generation,
            navigation_projection,
            navigation_enrichment,
            focused_projection_rx: Arc::new(Mutex::new(Some(focused_projection_rx))),
            #[cfg(any(test, feature = "test-hooks"))]
            residency_room_tx,
            #[cfg(any(test, feature = "test-hooks"))]
            residency_room_operation_reached_count,
            native_artifacts,
            account_work: handle_account_work,
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    async fn qa_set_local_device_blacklisted(
        &self,
        target: VerificationTarget,
        room_id: String,
    ) -> Result<(), ()> {
        let session = self.session.as_ref().ok_or(())?;
        let user_id = matrix_sdk::ruma::UserId::parse(target.user_id).map_err(|_| ())?;
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

    async fn run(mut self) {
        #[cfg(test)]
        let mut shutdown_gate = None;
        let mut shutdown_ack: Option<oneshot::Sender<bool>> = None;
        let mut demand_channel_open = true;
        loop {
            // Fair selection prevents a busy publisher from starving commands;
            // the check below still applies pending demand before a completion.
            let msg = tokio::select! {
                changed = self.avatar_demand_rx.changed(), if demand_channel_open => {
                    if changed.is_err() {
                        demand_channel_open = false;
                        self.avatar_demand = None;
                        self.reconcile_avatar_demand(false, false).await;
                    } else {
                        self.accept_avatar_demand().await;
                    }
                    continue;
                }
                msg = self.command_rx.recv() => match msg {
                    Some(msg) => msg,
                    None => break,
                },
            };
            // A removal can arrive concurrently with dequeuing a completion.
            if self.avatar_demand_rx.has_changed().unwrap_or(false) {
                Box::pin(self.accept_avatar_demand()).await;
            }
            // #1554: box each message's work so this loop's frame holds one
            // pointer per arm instead of every arm's temporary futures.
            match msg {
                #[cfg(test)]
                AccountMessage::StopRoomActorForTesting { acknowledged } => {
                    let _ = Box::pin(self.stop_room_actor()).await;
                    let _ = acknowledged.send(());
                }
                #[cfg(test)]
                AccountMessage::ConfigureShutdownGate { entered, release } => {
                    shutdown_gate = Some((entered, release));
                }
                #[cfg(test)]
                AccountMessage::Shutdown => {
                    Box::pin(self.abort_direct_searches()).await;
                    Box::pin(self.abort_direct_attachments()).await;
                    break;
                }
                AccountMessage::ShutdownWithAck { acknowledged } => {
                    Box::pin(self.abort_direct_searches()).await;
                    Box::pin(self.abort_direct_attachments()).await;
                    shutdown_ack = Some(acknowledged);
                    break;
                }
                AccountMessage::Command(command) => {
                    Box::pin(self.handle_command(command)).await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::QaSetLocalDeviceBlacklisted {
                    target,
                    room_id,
                    acknowledged,
                } => {
                    let result =
                        Box::pin(self.qa_set_local_device_blacklisted(target, room_id)).await;
                    let _ = acknowledged.send(result);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::QaRefreshDeviceKeysAndAssertKnown {
                    target,
                    acknowledged,
                } => {
                    let result = match self.session.as_ref() {
                        Some(session) => {
                            Box::pin(refresh_device_keys_and_assert_known(session, target)).await
                        }
                        None => Err(()),
                    };
                    let _ = acknowledged.send(result);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::QaAssertInboundSessionsStartAtZero {
                    room_id,
                    acknowledged,
                } => {
                    let result = match self.session.as_ref() {
                        Some(session) => match Box::pin(
                            koushi_sdk::inbound_group_session_index0_summary(session, &room_id),
                        )
                        .await
                        {
                            Ok((count, true)) if count > 0 => Ok(count),
                            _ => Err(()),
                        },
                        None => Err(()),
                    };
                    let _ = acknowledged.send(result);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::QaSyncOnce { request_id } => {
                    Box::pin(self.route_sync_once_for_qa(request_id)).await;
                }
                AccountMessage::ContinueSlidingSyncAdmission {
                    account_epoch,
                    request_id,
                    source,
                } => {
                    Box::pin(self.continue_sliding_sync_admission(
                        account_epoch,
                        request_id,
                        source,
                    ))
                    .await;
                }
                AccountMessage::RetrySlidingSyncCapabilityDiscovery {
                    account_epoch,
                    blocked_request_id,
                    request_id,
                } => {
                    Box::pin(self.start_sliding_sync_capability_retry(
                        account_epoch,
                        blocked_request_id,
                        request_id,
                    ))
                    .await;
                }
                AccountMessage::ScheduleSlidingSyncCapabilityRevalidation { account_epoch } => {
                    if account_epoch == self.sliding_sync_account_epoch {
                        self.sliding_sync_revalidation_pending = Some(account_epoch);
                        if self.session_promoted {
                            Box::pin(self.start_sliding_sync_revalidation(account_epoch)).await;
                        }
                    }
                }
                AccountMessage::SlidingSyncCapabilityDiscovered {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    Box::pin(self.finish_sliding_sync_capability_discovery(
                        account_epoch,
                        request_id,
                        result,
                    ))
                    .await;
                }
                AccountMessage::SlidingSyncCapabilityRevalidationDiscovered {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    Box::pin(self.finish_sliding_sync_revalidation(
                        account_epoch,
                        request_id,
                        result,
                    ))
                    .await;
                }
                AccountMessage::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    Box::pin(self.settle_sliding_sync_revalidation(
                        account_epoch,
                        request_id,
                        result,
                    ))
                    .await;
                }
                AccountMessage::SyncCommand(sync_command) => {
                    Box::pin(self.route_sync_command(sync_command)).await;
                }
                AccountMessage::RoomCommand(room_command) => {
                    Box::pin(self.route_room_command(room_command)).await;
                }
                AccountMessage::ReadReceiptWindow {
                    source,
                    start,
                    limit,
                    response,
                } => {
                    let _ = Box::pin(self.timeline_manager.send(
                        crate::timeline::TimelineMessage::ReadReceiptWindow {
                            source,
                            start,
                            limit,
                            response,
                        },
                    ))
                    .await;
                }
                AccountMessage::DirectAttachment {
                    room_id,
                    event_id,
                    thumbnail,
                    response,
                } => {
                    Box::pin(self.handle_direct_attachment(room_id, event_id, thumbnail, response))
                        .await;
                }
                AccountMessage::DirectSearch {
                    query,
                    room_ids,
                    content_policy,
                    response,
                } => {
                    Box::pin(self.handle_direct_search(query, room_ids, content_policy, response))
                        .await;
                }
                AccountMessage::DirectSessionAccessToken {
                    expected,
                    revision,
                    response,
                } => {
                    if !response.is_closed() {
                        let result = crate::session_access_token::read_current_session_access_token(
                            self.session.as_deref(),
                            &expected,
                            revision,
                        );
                        let _ = response.send(result);
                    }
                }
                AccountMessage::DirectRoomMessages {
                    room_id,
                    limit,
                    from,
                    response,
                } => {
                    Box::pin(self.handle_direct_room_messages(room_id, limit, from, response))
                        .await;
                }
                AccountMessage::DirectMessageAction { request, response } => {
                    Box::pin(self.handle_direct_message_action(request, response)).await;
                }
                AccountMessage::TimelineCommand(timeline_command) => {
                    Box::pin(self.route_timeline_command(timeline_command)).await;
                }
                AccountMessage::ReadStatePolicyChanged { send_read_receipts } => {
                    self.send_read_receipts = send_read_receipts;
                    let _ = Box::pin(self.timeline_manager.set_read_state_policy(
                        self.read_persistence_session_generation,
                        send_read_receipts,
                    ))
                    .await;
                }
                AccountMessage::DisplayPolicyChanged {
                    thread_root_order,
                    hide_redacted,
                } => {
                    let _ = Box::pin(
                        self.timeline_manager
                            .set_display_policy(thread_root_order, hide_redacted),
                    )
                    .await;
                }
                AccountMessage::TimelineCommandWithComposerFormatting {
                    command,
                    formatting_options,
                } => {
                    Box::pin(self.route_timeline_command_with_formatting_options(
                        command,
                        formatting_options,
                    ))
                    .await;
                }
                AccountMessage::LeasedTimelineCommandWithComposerFormatting {
                    command,
                    composer_permit,
                    formatting_options,
                } => {
                    Box::pin(self.route_leased_timeline_command_with_formatting_options(
                        command,
                        composer_permit,
                        formatting_options,
                    ))
                    .await;
                }
                AccountMessage::ResolveActivity {
                    generation,
                    requests,
                } => {
                    if let Some(task) = self.activity_resolution_task.take() {
                        task.abort();
                    }
                    let Some(session) = self.session.clone() else {
                        let _ = Box::pin(self.action_tx.send(vec![
                            AppAction::ActivityResolutionFailed {
                                generation,
                                unresolved_room_count:
                                    requests.len().try_into().unwrap_or(u32::MAX),
                                kind: OperationFailureKind::Sdk,
                            },
                        ]))
                        .await;
                        continue;
                    };
                    let action_tx = self.action_tx.clone();
                    let backpressure = self.account_work.clone();
                    self.activity_resolution_task = Some(crate::executor::spawn(async move {
                        let outcome =
                            Box::pin(crate::activity_resolution::resolve_activity_requests(
                                &session,
                                &requests,
                                &backpressure,
                            ))
                            .await;
                        let settlement = match outcome.failure_kind {
                            Some(kind) => AppAction::ActivityResolutionFailed {
                                generation,
                                unresolved_room_count: outcome.unresolved_room_count,
                                kind,
                            },
                            None => AppAction::ActivityResolutionSucceeded { generation },
                        };
                        let _ = Box::pin(action_tx.send(vec![
                            AppAction::ActivityResolutionRowsObserved {
                                generation,
                                rows: outcome.rows,
                            },
                            settlement,
                        ]))
                        .await;
                    }));
                }
                AccountMessage::ExportHistory {
                    request_id,
                    request,
                    locale,
                } => {
                    Box::pin(self.handle_export_history(request_id, request, locale)).await;
                }
                AccountMessage::CancelActivityResolution => {
                    if let Some(task) = self.activity_resolution_task.take() {
                        task.abort();
                    }
                }
                AccountMessage::ScheduleServerDelayedSend {
                    request_id,
                    expected_account,
                    scheduled_id,
                    room_id,
                    thread_root_event_id,
                    body,
                    send_at_ms,
                    draft_revision,
                    composer_permit,
                    formatting_options,
                } => {
                    Box::pin(self.handle_schedule_server_delayed_send(
                        request_id,
                        expected_account,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
                        send_at_ms,
                        draft_revision,
                        composer_permit,
                        formatting_options,
                    ))
                    .await;
                }
                AccountMessage::DispatchLocalScheduledSend {
                    request_id,
                    origin_session_key,
                    scheduled_id,
                    room_id,
                    thread_root_event_id,
                    body,
                    formatting_options,
                } => {
                    Box::pin(self.handle_dispatch_local_scheduled_send(
                        request_id,
                        origin_session_key,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
                        formatting_options,
                    ))
                    .await;
                }
                AccountMessage::CancelServerDelayedSend {
                    request_id,
                    scheduled_id,
                    delay_id,
                } => {
                    Box::pin(self.handle_cancel_server_delayed_send(
                        request_id,
                        scheduled_id,
                        delay_id,
                    ))
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
                    formatting_options,
                } => {
                    Box::pin(self.handle_reschedule_server_delayed_send(
                        request_id,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
                        delay_id,
                        send_at_ms,
                        formatting_options,
                    ))
                    .await;
                }
                AccountMessage::OpenTimelineAtTimestamp {
                    request_id,
                    room_id,
                    timestamp_ms,
                } => {
                    Box::pin(self.handle_open_timeline_at_timestamp(
                        request_id,
                        room_id,
                        timestamp_ms,
                    ))
                    .await;
                }
                #[cfg(test)]
                AccountMessage::ConfigureEventCacheFetchForTesting { fetch } => {
                    self.event_cache_fetch_override = Some(fetch);
                }
                #[cfg(test)]
                AccountMessage::NoteOwnAvatarSourceForTesting {
                    mxc_uri,
                    acknowledged,
                } => {
                    self.note_own_avatar_source(mxc_uri);
                    let _ = acknowledged.send(());
                }
                #[cfg(test)]
                AccountMessage::ConfigureSearchActorForTesting {
                    handle,
                    acknowledged,
                } => {
                    if let Some(handle) = handle {
                        self.search_actor = Some(handle);
                    }
                    let _ = acknowledged.send(());
                }
                AccountMessage::EnsureRoomEventCached {
                    request_id,
                    room_id,
                    event_id,
                    response_tx,
                } => {
                    let result = Box::pin(
                        self.handle_ensure_room_event_cached(request_id, room_id, event_id),
                    )
                    .await;
                    let _ = response_tx.send(result);
                }
                AccountMessage::RepairRoomTimeline {
                    request_id,
                    account_key,
                    room_id,
                } => {
                    Box::pin(self.route_timeline_command(TimelineCommand::RepairGaps {
                        request_id,
                        key: TimelineKey {
                            account_key,
                            kind: TimelineKind::Room { room_id },
                        },
                    }))
                    .await;
                }
                AccountMessage::SearchWithPolicy {
                    command,
                    content_policy,
                } => {
                    // A buffered roster must not restore an older policy after
                    // this authoritative submission crosses mailbox pressure.
                    if let Some(notification) = self.pending_crawler_notification.as_mut() {
                        notification.settings = content_policy.clone();
                    }
                    Box::pin(self.route_search_command_with_policy(command, Some(content_policy)))
                        .await;
                }
                AccountMessage::SearchCommand(search_command) => {
                    Box::pin(self.route_search_command(search_command)).await;
                }
                AccountMessage::SearchRetentionDiagnostics { response } => {
                    match &self.search_actor {
                        Some(search_actor) => {
                            // Forwarded; on a failed send the sender is consumed
                            // and the caller's bounded wait settles as
                            // unavailable.
                            let _ = Box::pin(search_actor.retention_diagnostics_forward(response))
                                .await;
                        }
                        None => {
                            // No search session for this account: settle as
                            // unavailable rather than holding the export.
                            let _ = response.send(None);
                        }
                    }
                }
                AccountMessage::TimelineResidencyDiagnostics { response } => {
                    // Forwarded; a closed manager drops the reply channel and the
                    // caller's bounded wait settles as unavailable.
                    self.timeline_manager.residency_diagnostics(response);
                }
                AccountMessage::NotifySearchCrawlerRoomsAvailable {
                    room_ids,
                    latest_event_ids,
                    settings,
                } => {
                    // Background lane: crawler room availability is
                    // latest-wins/coalesced/recoverable state. Store it first,
                    // then try a non-blocking flush so AccountActor never stalls
                    // user-intent or foreground room/timeline commands behind
                    // crawler mailbox pressure.
                    let notification = crate::search::CrawlerRoomsNotification {
                        room_ids,
                        latest_event_ids,
                        settings,
                    };
                    self.pending_crawler_notification = Some(notification);
                    self.flush_pending_crawler_notification();
                }
                AccountMessage::CurrentDeviceTrustChanged { generation, trust } => {
                    Box::pin(self.handle_observed_current_device_trust(generation, trust)).await;
                }
                AccountMessage::ActiveSessionAccountManagementUrlResolved {
                    generation,
                    info,
                    url,
                } => {
                    Box::pin(self.handle_active_session_account_management_url_resolved(
                        generation, info, url,
                    ))
                    .await;
                }
                #[cfg(test)]
                AccountMessage::ConfigureAccountManagementDiscovery { result } => {
                    *self
                        .account_management_discovery_override
                        .lock()
                        .expect("account-management discovery override lock") = Some(result);
                }
                AccountMessage::CheckCurrentDeviceTrust => {
                    self.request_authoritative_trust_recheck();
                }
                AccountMessage::RefreshCurrentSessionStatus {
                    request_id,
                    trigger,
                    sync_state,
                } => {
                    self.start_current_session_status_refresh(request_id, trigger, sync_state);
                }
                AccountMessage::ArmCurrentSessionStatusCheck { token, due_at_ms } => {
                    self.arm_current_session_status_timer(token, due_at_ms);
                }
                AccountMessage::CurrentSessionStatusTimerFired { token } => {
                    Box::pin(self.handle_current_session_status_timer_fired(token)).await;
                }
                AccountMessage::OwnSessionCheckDue { generation } => {
                    self.handle_own_session_check_due(generation);
                }
                AccountMessage::OwnSessionCheckFinished { generation, result } => {
                    Box::pin(self.handle_own_session_check_finished(generation, result)).await;
                }
                AccountMessage::TrustRecheckRetryDue { serial } => {
                    self.handle_trust_recheck_retry_due(serial);
                }
                #[cfg(test)]
                AccountMessage::InspectSessionCheckTimer { response } => {
                    let _ =
                        response.send(self.session_check.timer.as_ref().map(|(token, _)| *token));
                }
                #[cfg(test)]
                AccountMessage::InspectTrustRecheckJoined { response } => {
                    let _ = response.send(self.session_check.trust_joined_inspection);
                }
                #[cfg(test)]
                AccountMessage::InspectSdkDeviceTrust { response } => {
                    let _ = response.send(
                        self.session
                            .as_ref()
                            .map(|session| session.current_device_trust()),
                    );
                }
                #[cfg(test)]
                AccountMessage::InspectOwnIdentityWorkPending { response } => {
                    let _ = response.send(
                        self.trust_recheck_task.is_some()
                            || self.trust_recheck_pending
                            || self.current_session_status_task.is_some(),
                    );
                }
                #[cfg(test)]
                AccountMessage::ConfigureSessionCheckClock { base_epoch_ms } => {
                    self.session_check.clock = super::session_check::SessionCheckClock::Virtual {
                        base_epoch_ms,
                        origin: crate::executor::Instant::now(),
                    };
                }
                AccountMessage::CurrentSessionStatusRefreshFinished {
                    request_id,
                    generation,
                    sync_state,
                    started_at,
                    result,
                } => {
                    Box::pin(self.finish_current_session_status_refresh(
                        request_id, generation, sync_state, started_at, result,
                    ))
                    .await;
                }
                AccountMessage::CurrentDeviceTrustRecheckFinished { generation, result } => {
                    if generation != self.trust_generation {
                        continue;
                    }
                    let settled_stage = match &result {
                        Ok((koushi_state::CurrentDeviceTrustState::Verified, _)) => {
                            "trust_recheck_finished_verified"
                        }
                        Ok((koushi_state::CurrentDeviceTrustState::Unverified, _)) => {
                            "trust_recheck_finished_unverified"
                        }
                        Ok((koushi_state::CurrentDeviceTrustState::Unknown, _)) => {
                            "trust_recheck_finished_unknown"
                        }
                        Err(_) => "trust_recheck_finished_failed",
                    };
                    let mut settlement_event =
                        verification_admission_event(settled_stage, generation, 0);
                    if let Some(error) = result.as_ref().err() {
                        settlement_event = settlement_event.field(DiagnosticField::token(
                            "failure_kind",
                            current_device_trust_recheck_failure_token(error),
                        ));
                    }
                    record_verification_admission_event(settlement_event);
                    self.trust_recheck_task = None;
                    let replay_after_settlement = self.trust_recheck_pending;
                    self.trust_recheck_pending = false;
                    let recheck_succeeded = result.is_ok();
                    let confirmed_missing = matches!(
                        &result,
                        Ok((
                            koushi_state::CurrentDeviceTrustState::Unknown,
                            koushi_state::VerificationAccountKind::NewIdentity
                        ))
                    );
                    let trust = match result {
                        Ok((trust, _)) => Some(trust),
                        Err(_)
                            if self.session_promoted
                                || matches!(
                                    self.pending_trust_transition,
                                    Some(PendingTrustTransition {
                                        generation: pending_generation,
                                        decision: TrustLifecycleDecision::Promote,
                                        ..
                                    }) if pending_generation == generation
                                ) =>
                        {
                            None
                        }
                        Err(_) => Some(koushi_state::CurrentDeviceTrustState::Unknown),
                    };
                    if let Some(trust) = trust {
                        Box::pin(self.handle_current_device_trust(generation, trust)).await;
                    }
                    // Missing identity permits bootstrap, not a different SDK trust verdict.
                    if confirmed_missing
                        && self.session.is_some()
                        && !self.session_promoted
                        && self.provisional_encryption_sync_ready
                    {
                        Box::pin(self.send_actions(vec![
                            AppAction::VerificationMethodDiscoveryRetryStarted { generation },
                            AppAction::VerificationMethodsDiscovered(
                                koushi_state::VerificationGateState {
                                    account_kind:
                                        koushi_state::VerificationAccountKind::NewIdentity,
                                    methods: vec![
                                        koushi_state::VerificationMethodCapability::Bootstrap,
                                    ],
                                    failure: None,
                                },
                            ),
                        ]))
                        .await;
                    }
                    // A trust recheck follows own cross-signing changes
                    // (import, bootstrap, own verification), which decide
                    // whether Verify user can sign an open contact (#1024).
                    if recheck_succeeded {
                        Box::pin(self.refresh_any_open_contact_security()).await;
                    }
                    // #1009: a failure on a promoted session keeps the demand
                    // pending behind the shared failure backoff.
                    self.record_trust_recheck_settlement(recheck_succeeded);
                    // The waiting inspection starts first, so a replayed demand
                    // joins its own-identity query instead of overlapping it.
                    self.start_waiting_current_session_inspection(recheck_succeeded);
                    if replay_after_settlement {
                        self.trust_recheck_pending = true;
                        self.start_authoritative_trust_recheck_if_idle(true);
                    }
                }
                AccountMessage::FirstProvisionalEncryptionSyncFinished {
                    generation,
                    succeeded,
                } => {
                    if succeeded {
                        self.sliding_sync_diagnostics
                            .provisional_encryption_first_response_seen();
                    }
                    if first_provisional_encryption_sync_is_current(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                    ) {
                        Box::pin(self.cancel_verification_method_discovery_admission_timeout())
                            .await;
                        self.provisional_encryption_sync_ready = succeeded;
                        let trust = self
                            .session
                            .as_ref()
                            .map(|session| session.current_device_trust())
                            .unwrap_or(koushi_state::CurrentDeviceTrustState::Unknown);
                        if succeeded && should_discover_verification_methods(trust) {
                            Box::pin(self.discover_verification_methods(generation)).await;
                        } else if succeeded
                            && trust == koushi_state::CurrentDeviceTrustState::Verified
                        {
                            Box::pin(self.handle_current_device_trust(generation, trust)).await;
                        } else if trust == koushi_state::CurrentDeviceTrustState::Unknown {
                            self.request_authoritative_trust_recheck();
                        } else {
                            self.verification_method_discovery_failed = true;
                            Box::pin(self.send_actions(vec![
                                AppAction::VerificationMethodDiscoveryFailed {
                                    generation,
                                    kind: koushi_state::VerificationGateFailureKind::Sdk,
                                },
                            ]))
                            .await;
                        }
                    }
                }
                AccountMessage::VerificationMethodDiscoveryAdmissionTimedOut {
                    generation,
                    serial,
                } => {
                    if method_discovery_admission_timeout_is_current(
                        generation,
                        self.trust_generation,
                        serial,
                        self.verification_method_discovery_serial,
                        self.session.is_some(),
                        self.session_promoted,
                        self.verification_method_discovery_task.is_some(),
                    ) {
                        self.verification_method_discovery_admission_task.take();
                        self.verification_method_discovery_failed = true;
                        record_verification_method_discovery_event(
                            verification_method_discovery_event(
                                "admission_timeout",
                                generation,
                                serial,
                            ),
                        );
                        Box::pin(self.send_actions(vec![
                            AppAction::VerificationMethodDiscoveryFailed {
                                generation,
                                kind: koushi_state::VerificationGateFailureKind::Timeout,
                            },
                        ]))
                        .await;
                    }
                }
                AccountMessage::ProvisionalEncryptionSyncSucceeded { generation } => {
                    let own_flow_id = self
                        .own_user_verification
                        .as_ref()
                        .map(|(flow_id, _)| *flow_id);
                    if let Some(flow_id) = active_own_user_sas_flow_for_provisional_encryption_sync(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                        own_flow_id,
                    ) {
                        record_sas_verification_event(sas_verification_event(
                            "provisional_encryption_sync_succeeded",
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
                        Box::pin(self.recheck_own_user_sas_after_sync()).await;
                    }
                }
                AccountMessage::ProvisionalEncryptionSyncFailed { generation } => {
                    let own_flow_id = self
                        .own_user_verification
                        .as_ref()
                        .map(|(flow_id, _)| *flow_id);
                    if let Some(flow_id) = active_own_user_sas_flow_for_provisional_encryption_sync(
                        generation,
                        self.trust_generation,
                        self.session.is_some(),
                        self.session_promoted,
                        own_flow_id,
                    ) {
                        record_sas_verification_event(sas_verification_event(
                            "provisional_encryption_sync_failed",
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
                        let _ = self.verification_method_discovery_task.take();
                        match result {
                            VerificationMethodDiscoveryResult::Discovered(gate) => {
                                self.verification_method_discovery_failed = false;
                                if gate.account_kind
                                    == koushi_state::VerificationAccountKind::ExistingIdentity
                                    && gate.methods.is_empty()
                                {
                                    record_device_cleanup_offer("no_proof_method");
                                }
                                record_verification_method_discovery_event(
                                    verification_method_discovery_event(
                                        "success_projecting",
                                        generation,
                                        serial,
                                    )
                                    .field(
                                        DiagnosticField::count(
                                            "method_count",
                                            gate.methods.len() as u64,
                                        ),
                                    ),
                                );
                                Box::pin(self.send_actions(vec![
                                    AppAction::VerificationMethodsDiscovered(gate),
                                ]))
                                .await;
                                record_verification_method_discovery_event(
                                    verification_method_discovery_event(
                                        "success_projected",
                                        generation,
                                        serial,
                                    ),
                                );
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
                                record_device_cleanup_offer("recovery_failed");
                                Box::pin(self.send_actions(vec![
                                    AppAction::VerificationMethodDiscoveryFailed {
                                        generation,
                                        kind,
                                    },
                                ]))
                                .await;
                            }
                        }
                    } else if method_discovery_is_current(
                        generation,
                        self.trust_generation,
                        serial,
                        self.verification_method_discovery_serial,
                        self.session.is_some(),
                    ) {
                        self.verification_method_discovery_failed = true;
                        record_verification_method_discovery_event(
                            verification_method_discovery_event(
                                "completion_owner_missing",
                                generation,
                                serial,
                            ),
                        );
                        Box::pin(self.send_actions(vec![
                            AppAction::VerificationMethodDiscoveryFailed {
                                generation,
                                kind: koushi_state::VerificationGateFailureKind::Sdk,
                            },
                        ]))
                        .await;
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
                    Box::pin(
                        self.handle_recovery_finished(generation, flow_id, request_id, result),
                    )
                    .await;
                }
                AccountMessage::RecoveryTrustSettlementTimedOut {
                    generation,
                    flow_id,
                    request_id,
                    trust,
                } => {
                    Box::pin(self.handle_recovery_trust_settlement_timed_out(
                        generation, flow_id, request_id, trust,
                    ))
                    .await;
                }
                AccountMessage::TrustProjectionApplied {
                    generation,
                    transition_id,
                    ready,
                    locked,
                } => {
                    Box::pin(self.handle_trust_projection_applied(
                        generation,
                        transition_id,
                        ready,
                        locked,
                    ))
                    .await;
                }
                AccountMessage::InspectSecureBackup => {
                    self.start_secure_backup_inspection();
                }
                AccountMessage::SyncConnectivityChanged { proven } => {
                    Box::pin(self.handle_sync_connectivity_changed(proven)).await;
                }
                AccountMessage::SecureBackupDeferredDeadlineExpired {
                    generation,
                    deadline_serial,
                } => {
                    Box::pin(
                        self.handle_secure_backup_defer_deadline_expired(
                            generation,
                            deadline_serial,
                        ),
                    )
                    .await;
                }
                AccountMessage::SecureBackupInspectionFinished {
                    generation,
                    started_at,
                    result,
                } => {
                    Box::pin(self.finish_secure_backup_inspection(generation, started_at, result))
                        .await;
                }
                AccountMessage::RetrySecureBackupInspection {
                    generation,
                    monitor_serial,
                } => {
                    if !secure_backup_monitor_wakeup_is_current(
                        self.trust_generation,
                        self.secure_backup_monitor_serial,
                        self.session_promoted,
                        generation,
                        monitor_serial,
                    ) {
                        continue;
                    }
                    self.secure_backup_monitor_task.take();
                    self.start_secure_backup_inspection();
                }
                AccountMessage::SecureBackupStateChanged { generation, state } => {
                    Box::pin(self.handle_secure_backup_state_changed(generation, state)).await;
                }
                AccountMessage::RejectProvisionalSession { request_id } => {
                    Box::pin(self.perform_logout(request_id, true, false)).await;
                }
                AccountMessage::RetrySessionTeardown { generation } => {
                    Box::pin(self.retry_session_teardown(generation)).await;
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
                AccountMessage::InspectPendingDeviceCleanup { response } => {
                    let _ = response.send(self.pending_device_cleanup.is_some());
                }
                #[cfg(test)]
                AccountMessage::InspectSecureBackupOwners { response } => {
                    let _ = response.send(SecureBackupOwnersSnapshot {
                        inspection_pending: self.secure_backup_inspection_pending,
                        has_inspection_task: self.secure_backup_inspection_task.is_some(),
                        has_defer_deadline: self.secure_backup_defer_deadline_task.is_some(),
                        trust_generation: self.trust_generation,
                        defer_serial: self.secure_backup_defer_serial,
                        holds_revealed_recovery_key: self.revealed_recovery_key.is_some(),
                    });
                }
                #[cfg(test)]
                AccountMessage::ConfigureSecureBackupDeferWait { wait } => {
                    self.secure_backup_connectivity_wait = wait;
                }
                #[cfg(test)]
                AccountMessage::DisableSecureBackupObserver => {
                    self.secure_backup_observer_disabled = true;
                }
                #[cfg(test)]
                AccountMessage::SeedRevealedRecoveryKey { revealed } => {
                    self.revealed_recovery_key = Some(revealed);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::InspectSyncOwners { response } => {
                    let _ = response.send((
                        self.provisional_encryption_sync.is_some(),
                        false,
                        self.sync_actor.is_some(),
                    ));
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::SetCurrentDeviceTrustForTesting { trust } => {
                    Box::pin(self.handle_current_device_trust(self.trust_generation, trust)).await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestConfigureInstallGap {
                    reached,
                    release,
                    configured,
                } => {
                    self.residency_install_gap = Some((reached, release));
                    let _ = configured.send(());
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestConfigureTeardownGap {
                    reached,
                    release,
                    configured,
                } => {
                    self.residency_teardown_gap = Some((reached, release));
                    let _ = configured.send(());
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::PauseHistoryKeyImport {
                    disclosure,
                    reached,
                    resume,
                    installed,
                } => {
                    let _ = Box::pin(self.room_actor.send(
                        crate::room::RoomMessage::PauseHistoryKeyImport {
                            disclosure,
                            reached,
                            resume,
                            installed,
                        },
                    ))
                    .await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::HistoryRecoveryBarrier { room_id, completed } => {
                    let _ = Box::pin(self.room_actor.send(
                        crate::room::RoomMessage::HistoryRecoveryBarrier { room_id, completed },
                    ))
                    .await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestInstallSession { session, completed } => {
                    let installed = Box::pin(self.install_residency_test_session(session)).await;
                    let _ = completed.send(installed);
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestRoomCommand { command, accepted } => {
                    Box::pin(self.route_room_command(command)).await;
                    let _ = accepted.send(());
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestConfigureRoomOperation {
                    control,
                    configured,
                } => {
                    let _ = configured
                        .send(self.room_actor.install_room_operation_test_control(control));
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestTimelineSnapshot { response } => {
                    let _ = Box::pin(
                        self.timeline_manager
                            .residency_snapshot_for_testing(response),
                    )
                    .await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestTimelineGateSnapshot { response } => {
                    let _ =
                        response.send(self.timeline_manager.residency_gate_snapshot_for_testing());
                }
                #[cfg(any(test, feature = "test-hooks"))]
                AccountMessage::ResidencyTestShutdown { acknowledged } => {
                    shutdown_ack = Some(acknowledged);
                    break;
                }
                #[cfg(test)]
                AccountMessage::ConfigureSyntheticRecoveryTask { flow_id, pending } => {
                    Box::pin(self.stop_recovery_task()).await;
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
                AccountMessage::ConfigureRecoveryResult { completion } => {
                    *self
                        .recovery_result_override
                        .lock()
                        .expect("recovery result lock") = Some(completion);
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
                            let _ = Box::pin(request_stopped).await;
                        }),
                    });
                    let (sas_stop, sas_stopped) = oneshot::channel();
                    self.sas_verification_observer = Some(VerificationObservation {
                        stop_tx: sas_stop,
                        task: executor::spawn(async move {
                            let _ = Box::pin(sas_stopped).await;
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
                    Box::pin(self.settle_verification(flow_id, terminal)).await;
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
                    self.pending_oidc_login = Some(PendingOidcAttempt {
                        start_request_id,
                        flow: PendingOidcFlow::Synthetic { homeserver },
                    });
                    self.oidc_completion_override = Some(session);
                }
                #[cfg(test)]
                AccountMessage::ConfigurePendingOidc {
                    start_request_id,
                    homeserver,
                } => {
                    self.pending_oidc_login = Some(PendingOidcAttempt {
                        start_request_id,
                        flow: PendingOidcFlow::Synthetic { homeserver },
                    });
                }
                #[cfg(test)]
                AccountMessage::ConfigureCloseStoreResults { results } => {
                    self.close_store_results = results.into();
                }
                #[cfg(test)]
                AccountMessage::ConfigureDeviceCleanupResults { results } => {
                    self.device_cleanup_results = results.into();
                }
                AccountMessage::InvalidateSearchCrawlerCache => {
                    if let Some(handle) = &self.search_actor {
                        Box::pin(handle.invalidate_crawler_cache()).await;
                    }
                    // If the actor is not yet running there is no completed-room
                    // cache to clear; the pending_crawler_notification is
                    // already the latest settings so a new crawl will use them.
                }
                AccountMessage::RebuildSearchIndex => {
                    if let Some(handle) = &self.search_actor {
                        Box::pin(handle.rebuild_search_index()).await;
                    }
                }
                AccountMessage::ThreadsListCommand(threads_list_command) => {
                    Box::pin(self.route_threads_list_command(threads_list_command)).await;
                }
                AccountMessage::VerificationRequestProgress {
                    request_id,
                    target,
                    state,
                } => {
                    Box::pin(self.handle_verification_request_progress(request_id, target, state))
                        .await;
                }
                AccountMessage::SasVerificationProgress {
                    request_id,
                    target,
                    state,
                } => {
                    Box::pin(self.handle_sas_verification_progress(request_id, target, state))
                        .await;
                }
                AccountMessage::SasVerificationTimedOut { flow_id } => {
                    Box::pin(self.handle_sas_verification_timeout(flow_id)).await;
                }
                AccountMessage::ContactSecurityStoreChanged { generation } => {
                    Box::pin(self.handle_contact_security_store_changed(generation)).await;
                }
                AccountMessage::ContactSecurityLoadFinished {
                    request_id,
                    generation,
                    user_id,
                    result,
                    changes,
                } => {
                    Box::pin(self.handle_contact_security_load_finished(
                        request_id, generation, user_id, result, changes,
                    ))
                    .await;
                }
                AccountMessage::ContactUserVerificationRequestFinished {
                    request_id,
                    generation,
                    target,
                    result,
                } => {
                    Box::pin(self.handle_contact_user_verification_request_finished(
                        request_id, generation, target, result,
                    ))
                    .await;
                }
                AccountMessage::VerificationRequestObserverEnded { flow_id } => {
                    if self.active_verification_target(flow_id).is_some() {
                        record_sas_verification_event(
                            sas_verification_event("observer_ended", flow_id)
                                .field(DiagnosticField::token("observer", "request")),
                        );
                        Box::pin(self.settle_verification(
                            flow_id,
                            VerificationTerminal::Failed(TrustOperationFailureKind::Sdk),
                        ))
                        .await;
                    }
                }
                AccountMessage::SasVerificationObserverEnded { flow_id } => {
                    if self.active_verification_target(flow_id).is_some() {
                        record_sas_verification_event(
                            sas_verification_event("observer_ended", flow_id)
                                .field(DiagnosticField::token("observer", "sas")),
                        );
                        Box::pin(self.settle_verification(
                            flow_id,
                            VerificationTerminal::Failed(TrustOperationFailureKind::Sdk),
                        ))
                        .await;
                    }
                }
                AccountMessage::IncomingVerificationRequest {
                    session,
                    target,
                    handle,
                } => {
                    if incoming_verification_request_is_current(self.session.as_ref(), &session) {
                        let request_id = self.next_incoming_verification_request_id();
                        Box::pin(
                            self.handle_incoming_verification_request(request_id, target, handle),
                        )
                        .await;
                    } else {
                        record(incoming_verification_request_dropped_event(
                            self.session.is_some(),
                        ));
                    }
                }
                AccountMessage::SessionInvalidated { reason } => {
                    Box::pin(self.handle_session_invalidated(reason)).await;
                }
                AccountMessage::SessionTokensRefreshed => {
                    Box::pin(self.handle_session_tokens_refreshed()).await;
                }
                AccountMessage::IdentityResetAuthTimedOut { flow_id } => {
                    Box::pin(self.handle_identity_reset_auth_timeout(flow_id)).await;
                }
                #[cfg(test)]
                AccountMessage::AvatarFetchIdForTesting {
                    mxc_uri,
                    response_tx,
                } => {
                    let _ = response_tx.send(
                        self.avatar_fetch_abort_handles
                            .get(&mxc_uri)
                            .map(|handle| handle.id()),
                    );
                }
                AccountMessage::AvatarFetched {
                    mxc_uri,
                    generation,
                    fetch_id,
                    thumbnail,
                } => {
                    Box::pin(self.handle_avatar_fetched(mxc_uri, generation, fetch_id, thumbnail))
                        .await;
                }
                AccountMessage::AccountHydrationLoaded {
                    generation,
                    actions,
                    ignored_user_ids,
                } => {
                    Box::pin(self.handle_account_hydration_loaded(
                        generation,
                        actions,
                        ignored_user_ids,
                    ))
                    .await;
                }
            }
            self.flush_pending_crawler_notification();
        }
        #[cfg(test)]
        if let Some((entered, release)) = shutdown_gate {
            let _ = entered.send(());
            let _ = Box::pin(release).await;
        }
        let runtime_ok = Box::pin(self.shutdown_owned_runtime()).await;
        // Always attempt RoomActor cleanup, even when session teardown failed.
        let room_ok = Box::pin(self.stop_room_actor()).await;
        if let Some(acknowledged) = shutdown_ack {
            let _ = acknowledged.send(runtime_ok && room_ok);
        }
    }

    /// Start one direct (non-GUI) attachment fetch (#1524).
    async fn handle_direct_attachment(
        &mut self,
        room_id: String,
        event_id: String,
        thumbnail: bool,
        mut response: tokio::sync::oneshot::Sender<
            Result<
                crate::direct_operations::FetchedAttachment,
                crate::direct_operations::AttachmentFetchFailure,
            >,
        >,
    ) {
        let Some(session) = self.session.clone() else {
            let _ = response.send(Err(
                crate::direct_operations::AttachmentFetchFailure::AccountInactive,
            ));
            return;
        };
        // Downloads belong to the account's own cache, the same directory the
        // GUI's media cache maps for this tab. The app-level data dir would
        // write outside it.
        let data_dir = match &self.session_key_id {
            Some(key_id) => self.store.account_local_data_dir(key_id),
            None => self.data_dir.clone(),
        };
        let selection = if thumbnail {
            koushi_protocol::command::MediaDownloadSelection::Thumbnail {
                width: 800,
                height: 800,
            }
        } else {
            koushi_protocol::command::MediaDownloadSelection::File
        };
        self.direct_attachment_tasks
            .retain(|task| !task.is_finished());
        self.direct_attachment_tasks
            .push(crate::executor::spawn(async move {
                let fetch = crate::timeline::fetch_attachment(
                    session,
                    data_dir,
                    &room_id,
                    &event_id,
                    selection,
                );
                tokio::pin!(fetch);
                tokio::select! {
                    outcome = &mut fetch => {
                        if !response.is_closed() {
                            let _ = response.send(outcome.map(|cached| crate::direct_operations::FetchedAttachment {
                                path: cached.path,
                                mime_type: cached.mime_type,
                                size_bytes: cached.size_bytes,
                            }).map_err(|failure| match failure {
                                crate::timeline::AttachmentFailure::RoomNotFound => {
                                    crate::direct_operations::AttachmentFetchFailure::RoomNotFound
                                }
                                crate::timeline::AttachmentFailure::EventNotFound => {
                                    crate::direct_operations::AttachmentFetchFailure::EventNotFound
                                }
                                crate::timeline::AttachmentFailure::Unsupported => {
                                    crate::direct_operations::AttachmentFetchFailure::Unsupported
                                }
                                crate::timeline::AttachmentFailure::DownloadFailed => {
                                    crate::direct_operations::AttachmentFetchFailure::DownloadFailed
                                }
                            }));
                        }
                    }
                    _ = response.closed() => {}
                }
            }));
    }

    async fn abort_direct_attachments(&mut self) {
        for task in self.direct_attachment_tasks.drain(..) {
            task.abort();
            let _ = Box::pin(task).await;
        }
    }

    /// Start one direct (non-GUI) search (#1524).
    ///
    /// The result travels only on the caller's reply channel: no `AppAction` is
    /// reduced and no `CoreEvent` is emitted, so the GUI search pane and the
    /// GUI's own query lifetime are untouched.
    async fn handle_direct_search(
        &mut self,
        query: String,
        room_ids: Vec<String>,
        content_policy: koushi_state::SearchCrawlerSettings,
        mut response: tokio::sync::oneshot::Sender<
            Result<
                Vec<koushi_protocol::event::SearchResultItem>,
                crate::search::DirectSearchFailure,
            >,
        >,
    ) {
        let Ok(permit) = Arc::clone(&self.direct_search_slots).try_acquire_owned() else {
            let _ = response.send(Err(crate::search::DirectSearchFailure::Capacity));
            return;
        };
        let Some(session) = self.session.clone() else {
            let _ = response.send(Err(crate::search::DirectSearchFailure::AccountInactive));
            return;
        };
        self.direct_search_tasks.retain(|task| !task.is_finished());
        self.direct_search_tasks
            .push(crate::executor::spawn(async move {
                let _permit = permit;
                let search =
                    crate::search::run_direct_search(session, query, room_ids, content_policy);
                tokio::pin!(search);
                tokio::select! {
                    outcome = &mut search => {
                        if !response.is_closed() {
                            let _ = response.send(outcome);
                        }
                    }
                    _ = response.closed() => {}
                }
            }));
    }

    /// Runs one direct (non-GUI) message read or action for an out-of-band
    /// caller (#1628).
    ///
    /// The session owns the room lookup and the bounded read, so no timeline
    /// actor is mounted and no app action or GUI event is produced. Reaction
    /// decisions take the per-account reaction lane, because deciding "no own
    /// reaction exists" and sending it must not interleave with another call.
    async fn handle_direct_message_action(
        &mut self,
        request: crate::direct_operations::DirectMessageRequest,
        response: tokio::sync::oneshot::Sender<
            Result<
                crate::direct_operations::DirectMessageResponse,
                crate::direct_operations::DirectMessageFailure,
            >,
        >,
    ) {
        use crate::direct_operations::DirectMessageFailure as Failure;
        let Some(session) = self.session.clone() else {
            let _ = response.send(Err(Failure::AccountInactive));
            return;
        };
        // A repeated retry key is answered from the ledger before a slot is
        // taken; a fresh claim is settled by the task on every path.
        let admitted = match super::direct_action_ledger::admit(
            &self.direct_action_ledger,
            &self.direct_message_action_slots,
            &request,
            crate::executor::Instant::now(),
        ) {
            Ok(admitted) => admitted,
            Err(answer) => {
                let _ = response.send(*answer);
                return;
            }
        };
        let lane = Arc::clone(&self.direct_reaction_lane);
        let ledger = Arc::clone(&self.direct_action_ledger);
        self.direct_message_action_tasks
            .retain(|task| !task.is_finished());
        self.direct_message_action_tasks
            .push(crate::executor::spawn(async move {
                let _permit = admitted.permit;
                let outcome = super::direct_action_ledger::run_claimed(
                    &ledger,
                    admitted.claim,
                    DIRECT_MESSAGE_ACTION_TIMEOUT,
                    Box::pin(crate::timeline::run_direct_message_action(
                        session, lane, request,
                    )),
                )
                .await;
                if !response.is_closed() {
                    let _ = response.send(outcome);
                }
            }));
    }

    async fn abort_direct_searches(&mut self) {
        for task in self.direct_search_tasks.drain(..) {
            task.abort();
            let _ = Box::pin(task).await;
        }
        for task in self.direct_room_messages_tasks.drain(..) {
            task.abort();
            let _ = Box::pin(task).await;
        }
        for task in self.direct_message_action_tasks.drain(..) {
            task.abort();
            let _ = Box::pin(task).await;
        }
    }

    /// Read one page of a room's recent timeline for an out-of-band caller
    /// (#1530). The session resolves the room, so no timeline actor is mounted.
    async fn handle_direct_room_messages(
        &mut self,
        room_id: String,
        limit: u32,
        from: Option<String>,
        mut response: tokio::sync::oneshot::Sender<
            Result<
                koushi_protocol::event::RoomMessagesPage,
                crate::direct_operations::RoomMessagesFailure,
            >,
        >,
    ) {
        let Some(session) = self.session.clone() else {
            let _ = response.send(Err(
                crate::direct_operations::RoomMessagesFailure::AccountInactive,
            ));
            return;
        };
        self.direct_room_messages_tasks
            .retain(|task| !task.is_finished());
        self.direct_room_messages_tasks
            .push(crate::executor::spawn(async move {
                let read = crate::timeline::read_room_messages(session, &room_id, limit, from);
                tokio::pin!(read);
                tokio::select! {
                    outcome = &mut read => {
                        if !response.is_closed() {
                            let _ = response.send(outcome);
                        }
                    }
                    _ = response.closed() => {}
                }
            }));
    }

    async fn handle_command(&mut self, command: AccountCommand) {
        match command {
            AccountCommand::DiscoverLogin {
                request_id,
                homeserver,
            } => {
                Box::pin(self.handle_discover_login(request_id, homeserver)).await;
            }
            AccountCommand::StartOidcLogin {
                request_id,
                homeserver,
                method,
            } => {
                Box::pin(self.handle_start_oidc_login(request_id, homeserver, method)).await;
            }
            AccountCommand::CancelOidcLogin { .. } => {
                self.handle_cancel_oidc_login();
            }
            AccountCommand::CompleteOidcLogin {
                request_id,
                callback_url,
                platform,
            } => {
                Box::pin(self.handle_complete_oidc_login(request_id, callback_url, platform)).await;
            }
            AccountCommand::LoginPassword {
                request_id,
                request,
                platform,
            } => {
                Box::pin(self.handle_login_password(request_id, request, platform)).await;
            }
            AccountCommand::RestoreSession {
                request_id,
                account_key,
            } => {
                Box::pin(self.handle_restore_session(request_id, account_key)).await;
            }
            AccountCommand::RestoreLastSession { request_id } => {
                Box::pin(self.handle_restore_last_session(request_id)).await;
            }
            AccountCommand::RetrySlidingSyncCapability { request_id } => {
                Box::pin(self.handle_retry_sliding_sync_capability(request_id)).await;
            }
            AccountCommand::QuerySavedSessions { request_id } => {
                Box::pin(self.handle_query_saved_sessions(request_id)).await;
            }
            AccountCommand::RefreshCurrentSessionStatus { .. } => {
                // The AppActor routes the reducer-owned refresh effect with
                // the authoritative sync projection.
            }
            AccountCommand::LoadAccountManagementCapabilities { request_id } => {
                Box::pin(self.handle_load_account_management_capabilities(request_id)).await;
            }
            AccountCommand::ChangePassword {
                request_id,
                new_password,
            } => {
                Box::pin(self.handle_change_password(request_id, new_password)).await;
            }
            AccountCommand::DeactivateAccount {
                request_id,
                erase_data,
                password,
            } => {
                Box::pin(self.handle_deactivate_account(request_id, erase_data, password)).await;
            }
            AccountCommand::SubmitAccountManagementUia {
                request_id,
                flow_id,
                auth,
            } => {
                Box::pin(self.handle_submit_account_management_uia(request_id, flow_id, auth))
                    .await;
            }
            AccountCommand::AccountNotifications {
                request_id,
                request,
            } => {
                Box::pin(self.handle_account_notifications(request_id, request)).await;
            }
            AccountCommand::ContactSecurity {
                request_id,
                request,
            } => {
                Box::pin(self.handle_contact_security(request_id, request)).await;
            }
            AccountCommand::SoftLogoutReauth {
                request_id,
                password,
            } => {
                Box::pin(self.handle_soft_logout_reauth(request_id, password)).await;
            }
            AccountCommand::ExportRoomKeys {
                request_id,
                request,
            } => {
                Box::pin(self.handle_export_room_keys(request_id, request)).await;
            }
            AccountCommand::ExportHistory { request_id, .. } => {
                // AppActor routes this command as `AccountMessage::ExportHistory`
                // with its resolved locale; a bare command cannot be exported.
                self.native_artifacts
                    .unregister(request_id, NativeArtifactKind::HistoryExportDirectory);
                Box::pin(self.send_actions(vec![AppAction::HistoryExportFailed {
                    request_id: request_id.sequence,
                    kind: koushi_state::HistoryExportFailureKind::Sdk,
                }]))
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed {
                        kind: koushi_protocol::failure::RoomFailureKind::Sdk,
                    },
                );
            }
            AccountCommand::StopHistoryExport {
                request_id,
                target_request_id,
            } => {
                Box::pin(self.handle_stop_history_export(request_id, target_request_id)).await;
            }
            AccountCommand::RetryHistoryExport {
                request_id,
                target_request_id,
            } => {
                Box::pin(self.handle_retry_history_export(request_id, target_request_id)).await;
            }
            AccountCommand::ImportRoomKeys {
                request_id,
                request,
            } => {
                Box::pin(self.handle_import_room_keys(request_id, request)).await;
            }
            AccountCommand::BootstrapSecureBackup {
                request_id,
                request,
            } => {
                Box::pin(self.handle_bootstrap_secure_backup(request_id, request)).await;
            }
            AccountCommand::RecoverSecureBackup {
                request_id,
                request,
            } => {
                Box::pin(self.handle_recover_secure_backup(request_id, request)).await;
            }
            AccountCommand::RetrySecureBackupInspection { request_id } => {
                if self.session_promoted && !self.sync_connectivity_proven {
                    // The deferred wait may have missed the proven edge (#860):
                    // ask the sync owner to re-project its current status so a
                    // healthy Running sync can still admit the inspection.
                    // Start re-projects without restarting a Running sync.
                    Box::pin(self.route_sync_command(SyncCommand::Start { request_id })).await;
                }
                self.start_secure_backup_inspection();
            }
            AccountCommand::ChangeSecureBackupPassphrase {
                request_id,
                request,
            } => {
                Box::pin(self.handle_change_secure_backup_passphrase(request_id, request)).await;
            }
            AccountCommand::SaveSecureBackupRecoveryKey {
                request_id,
                reveal_request_id,
            } => {
                Box::pin(
                    self.handle_save_secure_backup_recovery_key(request_id, reveal_request_id),
                )
                .await;
            }
            AccountCommand::ConfirmSecureBackupRecoveryKeySaved {
                request_id,
                reveal_request_id,
            } => {
                Box::pin(self.handle_confirm_secure_backup_recovery_key_saved(
                    request_id,
                    reveal_request_id,
                ))
                .await;
            }
            AccountCommand::ProbeLocalEncryptionHealth { request_id } => {
                Box::pin(self.handle_probe_local_encryption_health(request_id)).await;
            }
            AccountCommand::ResetLocalData { request_id } => {
                Box::pin(self.handle_reset_local_data(request_id)).await;
            }
            AccountCommand::StartDeviceCleanup { request_id } => {
                Box::pin(self.handle_start_device_cleanup(request_id)).await;
            }
            AccountCommand::SubmitDeviceCleanupUia {
                request_id,
                flow_id,
                password,
            } => {
                Box::pin(self.handle_submit_device_cleanup_uia(request_id, flow_id, password))
                    .await;
            }
            AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id } => {
                Box::pin(self.handle_erase_device_cleanup_local_data_anyway(request_id)).await;
            }
            AccountCommand::Logout { request_id } => {
                Box::pin(self.handle_logout(request_id)).await;
            }
            AccountCommand::ChangeHomeserver { request_id } => {
                Box::pin(self.handle_change_homeserver(request_id)).await;
            }
            AccountCommand::SwitchAccount {
                request_id,
                account_key,
            } => {
                Box::pin(self.handle_switch_account(request_id, account_key)).await;
            }
            AccountCommand::SubmitRecovery {
                request_id,
                request,
            } => {
                Box::pin(self.handle_submit_recovery(request_id, request)).await;
            }
            AccountCommand::StartSessionBootstrap {
                request_id,
                flow_id,
                auth,
                passphrase,
            } => {
                Box::pin(
                    self.handle_start_session_bootstrap(request_id, flow_id, auth, passphrase),
                )
                .await;
            }
            AccountCommand::ConfirmSessionBootstrapSaved {
                request_id,
                flow_id,
            } => {
                Box::pin(self.handle_confirm_session_bootstrap_saved(request_id, flow_id)).await;
            }
            AccountCommand::BootstrapCrossSigning { request_id, auth } => {
                Box::pin(self.handle_bootstrap_cross_signing(request_id, auth)).await;
            }
            AccountCommand::EnableKeyBackup {
                request_id,
                passphrase,
            } => {
                Box::pin(self.handle_enable_key_backup(request_id, passphrase)).await;
            }
            AccountCommand::RestoreKeyBackup {
                request_id,
                version,
                request,
            } => {
                Box::pin(self.handle_restore_key_backup(request_id, version, request)).await;
            }
            AccountCommand::ResetIdentity { request_id } => {
                Box::pin(self.handle_reset_identity(request_id)).await;
            }
            AccountCommand::CancelIdentityReset {
                request_id,
                flow_id,
            } => {
                Box::pin(self.handle_cancel_identity_reset(request_id, flow_id)).await;
            }
            AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request,
            } => {
                Box::pin(self.handle_submit_identity_reset_auth(request_id, flow_id, request))
                    .await;
            }
            AccountCommand::SetPresence {
                request_id,
                presence,
            } => {
                Box::pin(self.handle_set_presence(request_id, presence)).await;
            }
            AccountCommand::SetDisplayName {
                request_id,
                display_name,
            } => {
                Box::pin(self.handle_set_display_name(request_id, display_name)).await;
            }
            AccountCommand::SetLocalUserAlias {
                request_id,
                user_id,
                alias,
            } => {
                Box::pin(self.handle_set_local_user_alias(request_id, user_id, alias)).await;
            }
            AccountCommand::SetAvatar {
                request_id,
                request,
            } => {
                Box::pin(self.handle_set_avatar(request_id, request)).await;
            }
            AccountCommand::DownloadAvatarThumbnail {
                request_id,
                mxc_uri,
            } => {
                Box::pin(self.handle_download_avatar_thumbnail(request_id, mxc_uri)).await;
            }
            AccountCommand::CancelAvatarThumbnail {
                request_id,
                target_request_id,
                mxc_uri,
            } => {
                if request_id.connection_id == target_request_id.connection_id {
                    self.cancel_avatar_thumbnail(target_request_id, &mxc_uri);
                }
            }
            AccountCommand::IgnoreUser {
                request_id,
                user_id,
            } => {
                Box::pin(self.handle_ignore_user(request_id, user_id, true)).await;
            }
            AccountCommand::UnignoreUser {
                request_id,
                user_id,
            } => {
                Box::pin(self.handle_ignore_user(request_id, user_id, false)).await;
            }
            AccountCommand::ReportUser {
                request_id,
                user_id,
                reason,
            } => {
                Box::pin(self.handle_report_user(request_id, user_id, reason)).await;
            }
            AccountCommand::RequestVerification { request_id, target } => {
                Box::pin(self.handle_request_verification(request_id, target)).await;
            }
            AccountCommand::StartOwnUserSas {
                request_id,
                flow_id,
            } => {
                Box::pin(self.handle_start_own_user_sas(request_id, flow_id)).await;
            }
            AccountCommand::RetryCurrentDeviceTrustDiscovery { request_id: _ } => {
                let current_trust = self
                    .session
                    .as_ref()
                    .map(|session| session.current_device_trust());
                if retry_should_restart_method_discovery(self.session_promoted, current_trust) {
                    Box::pin(self.send_actions(vec![
                        AppAction::VerificationMethodDiscoveryRetryStarted {
                            generation: self.trust_generation,
                        },
                    ]))
                    .await;
                    Box::pin(self.discover_verification_methods(self.trust_generation)).await;
                } else {
                    self.request_authoritative_trust_recheck();
                }
            }
            AccountCommand::AcceptVerification {
                request_id,
                flow_id,
            } => {
                Box::pin(self.handle_accept_verification(request_id, flow_id)).await;
            }
            AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            } => {
                Box::pin(self.handle_confirm_sas_verification(request_id, flow_id)).await;
            }
            AccountCommand::CancelVerification {
                request_id,
                flow_id,
                reason,
            } => {
                Box::pin(self.handle_cancel_verification(request_id, flow_id, reason)).await;
            }
        }
    }

    pub(super) fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Announces admission for each held `LoggedIn`; the held event itself
    /// still waits for trust promotion.
    pub(super) fn emit_login_admitted(&self, ready_events: &[CoreEvent]) {
        for event in ready_events {
            if let CoreEvent::Account(koushi_protocol::event::AccountEvent::LoggedIn {
                request_id,
                account_key,
            }) = event
            {
                self.emit(CoreEvent::Account(
                    koushi_protocol::event::AccountEvent::LoginAdmitted {
                        request_id: *request_id,
                        account_key: account_key.clone(),
                    },
                ));
            }
        }
    }

    pub(super) fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }

    pub(super) fn emit_event_cache_status(
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

    pub(super) fn active_account_key(&self) -> Option<AccountKey> {
        self.session
            .as_ref()
            .map(|session| AccountKey(session.info.user_id.clone()))
    }

    pub(super) async fn send_actions(&self, actions: Vec<AppAction>) -> bool {
        self.action_tx.send(actions).await.is_ok()
    }
}

#[cfg(test)]
mod tests {

    use super::trace_account_request;

    use koushi_protocol::ids::{RequestId, RuntimeConnectionId};

    #[test]
    fn account_trace_preserves_typed_request_fields_without_environment_switch() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        trace_account_request(
            "test_account_typed_fields",
            RequestId {
                connection_id: RuntimeConnectionId(7),
                sequence: 11,
            },
            "restore_session",
        );
        let records = koushi_diagnostics::test_support::detail_snapshot()
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
}
