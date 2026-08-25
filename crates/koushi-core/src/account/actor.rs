//! `actor` ownership for AccountActor.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "test-hooks")]
use std::sync::atomic::AtomicUsize;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::SessionKeyId;
use koushi_sdk::{MatrixClientSession, PersistableMatrixSession};
#[cfg(test)]
use koushi_state::DeviceCleanupFailureKind;
use koushi_state::{
    AppAction, AvatarThumbnailState, ComposerDraftRevision, OperationFailureKind,
    SlidingSyncAdmissionSource, SlidingSyncCapabilityResult, SlidingSyncPositiveEvidence,
    TrustOperationFailureKind, VerificationTarget,
};
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};

use crate::command::{
    AccountCommand, RoomCommand, SearchCommand, SyncCommand, ThreadsListCommand, TimelineCommand,
};
use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
use crate::event::{
    CoreEvent, EventCacheFailureReasonClass, EventCacheSubscribeStatus, LocalEncryptionEvent,
};
#[cfg(test)]
use crate::executor;
use crate::failure::CoreFailure;
use crate::ids::{
    AccountKey, RequestId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind,
};
use crate::link_preview::LinkPreviewContext;
#[cfg(feature = "test-hooks")]
use crate::room::RoomOperationTestControl;
use crate::room::{RoomActorHandle, RoomMessage};
use crate::runtime::ForwardedComposerDraftPermit;
use crate::search::SearchActorHandle;
use crate::store::StoreActor;
use crate::sync::SyncActorHandle;
use crate::timeline::{
    NavigationProjectionIngress, NavigationProjectionIntent, TimelineManagerHandle,
    TimelineMessage, TimelineProjectionAcknowledgement,
};

use super::account_management::PendingUiaOperation;
use super::local_data_cleanup::{PendingDeviceCleanup, record_device_cleanup_offer};
use super::profile::AVATAR_DOWNLOAD_CONCURRENCY;
use super::recovery_backup::{
    PendingRecoveryCompletion, PendingRecoveryTask, secure_backup_monitor_wakeup_is_current,
};
use super::session_lifecycle::{
    PendingOidcFlow, PendingSessionTeardown, SessionChangeObservation, SessionInvalidationReason,
};
use super::sliding_sync::{
    PendingSlidingSyncAdmission, PendingSlidingSyncRetry, StoredSlidingSyncAdmissionContext,
};
#[cfg(feature = "qa-bin")]
use super::trust_gate::refresh_device_keys_and_assert_known;
use super::trust_gate::{
    OwnedVerificationMethodDiscoveryTask, PendingTrustTransition, RecoveryStateObservation,
    TrustLifecycleDecision, VerificationMethodDiscoveryResult,
    active_own_user_sas_flow_for_provisional_encryption_sync,
    current_device_trust_recheck_failure_token, first_provisional_encryption_sync_is_current,
    method_discovery_is_current, own_user_sas_recheck_is_current,
    record_verification_admission_event, record_verification_method_discovery_event,
    retry_should_restart_method_discovery, should_discover_verification_methods,
    verification_admission_event, verification_gate_failure_token,
    verification_method_discovery_event,
};
use super::verification::{
    INCOMING_VERIFICATION_FLOW_ID_BASE, IncomingVerificationObservation, PendingSasVerification,
    PendingVerificationRequest, SasVerificationWaitState, VerificationObservation,
    VerificationTerminal, incoming_verification_request_is_current, record_sas_verification_event,
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

/// Messages routed to the AccountActor task.
pub(crate) enum AccountMessage {
    Command(AccountCommand),
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
    TimelineCommandWithComposerFormatting {
        command: TimelineCommand,
        formatting_options: koushi_state::ComposerFormattingOptions,
    },
    LeasedTimelineCommand {
        command: TimelineCommand,
        composer_permit: ForwardedComposerDraftPermit,
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
    AcknowledgeTimelineProjection {
        projection_request_id: RequestId,
        key: TimelineKey,
        generation: TimelineGeneration,
        response: oneshot::Sender<TimelineProjectionAcknowledgement>,
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
        expected_account: SessionKeyId,
        scheduled_id: String,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        send_at_ms: u64,
        draft_revision: ComposerDraftRevision,
        composer_permit: ForwardedComposerDraftPermit,
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
    CheckCurrentDeviceTrust,
    InspectSecureBackup,
    SecureBackupInspectionFinished {
        generation: u64,
        result: Result<
            koushi_sdk::MatrixSecureBackupInspection,
            koushi_state::SecureBackupGateFailureKind,
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
            koushi_state::CurrentDeviceTrustState,
            koushi_sdk::CurrentDeviceTrustRecheckError,
        >,
    },
    FirstProvisionalEncryptionSyncFinished {
        generation: u64,
        succeeded: bool,
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
    #[cfg(feature = "test-hooks")]
    ResidencyTestConfigureInstallGap {
        reached: oneshot::Sender<(
            Option<Arc<MatrixClientSession>>,
            Option<Arc<MatrixClientSession>>,
        )>,
        release: oneshot::Receiver<()>,
        configured: oneshot::Sender<()>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestConfigureTeardownGap {
        reached: oneshot::Sender<bool>,
        release: oneshot::Receiver<()>,
        configured: oneshot::Sender<()>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestInstallSession {
        session: Arc<MatrixClientSession>,
        completed: oneshot::Sender<()>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestRoomCommand {
        command: RoomCommand,
        accepted: oneshot::Sender<()>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestConfigureRoomOperation {
        control: RoomOperationTestControl,
        configured: oneshot::Sender<bool>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestTimelineSnapshot {
        response: oneshot::Sender<(Vec<String>, Vec<String>)>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestTimelineGateSnapshot {
        response: oneshot::Sender<(bool, usize)>,
    },
    #[cfg(feature = "test-hooks")]
    ResidencyTestShutdown {
        acknowledged: oneshot::Sender<()>,
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
    ConfigureCloseStoreResults {
        results: Vec<bool>,
    },
    #[cfg(test)]
    ConfigureDeviceCleanupResults {
        results: Vec<Result<koushi_sdk::MatrixDeviceCleanupOutcome, DeviceCleanupFailureKind>>,
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
        reason: SessionInvalidationReason,
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
    navigation_projection: NavigationProjectionIngress,
    #[cfg(feature = "test-hooks")]
    residency_room_tx: mpsc::Sender<RoomMessage>,
    #[cfg(feature = "test-hooks")]
    residency_room_operation_reached_count: Arc<AtomicUsize>,
}

impl AccountActorHandle {
    pub(crate) async fn send(&self, msg: AccountMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    #[cfg(feature = "test-hooks")]
    pub async fn configure_residency_install_gap(
        &self,
        reached: oneshot::Sender<(
            Option<Arc<MatrixClientSession>>,
            Option<Arc<MatrixClientSession>>,
        )>,
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

    #[cfg(feature = "test-hooks")]
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

    #[cfg(feature = "test-hooks")]
    pub async fn install_residency_test_session(&self, session: Arc<MatrixClientSession>) -> bool {
        let (completed, acknowledged) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestInstallSession { session, completed })
            .await
        {
            return false;
        }
        acknowledged.await.is_ok()
    }

    #[cfg(feature = "test-hooks")]
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

    #[cfg(feature = "test-hooks")]
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
    #[cfg(feature = "test-hooks")]
    pub async fn residency_test_room_command_direct(&self, command: RoomCommand) -> bool {
        self.residency_room_tx
            .send(RoomMessage::Command(command))
            .await
            .is_ok()
    }

    /// Send a room command directly to the real RoomActor while the account
    /// actor is held at the install-gap barrier. This is only needed to probe
    /// the deliberate install→SessionEstablished window.
    #[cfg(feature = "test-hooks")]
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

    #[cfg(feature = "test-hooks")]
    pub fn residency_test_room_operation_reached_count(&self) -> usize {
        self.residency_room_operation_reached_count
            .load(Ordering::SeqCst)
    }

    #[cfg(feature = "test-hooks")]
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

    #[cfg(feature = "test-hooks")]
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

    #[cfg(feature = "test-hooks")]
    pub async fn shutdown_for_testing(&self) -> bool {
        let (acknowledged, completion) = oneshot::channel();
        if !self
            .send(AccountMessage::ResidencyTestShutdown { acknowledged })
            .await
        {
            return false;
        }
        completion.await.is_ok()
    }

    pub(crate) fn admit_navigation_projection(&self, intent: NavigationProjectionIntent) -> bool {
        self.navigation_projection.admit(intent)
    }

    #[cfg(test)]
    pub(crate) fn for_app_actor_test(
        tx: mpsc::Sender<AccountMessage>,
        navigation_projection: NavigationProjectionIngress,
    ) -> Self {
        Self {
            tx,
            navigation_projection,
            #[cfg(feature = "test-hooks")]
            residency_room_tx: {
                let (room_tx, _room_rx) = mpsc::channel(1);
                room_tx
            },
            #[cfg(feature = "test-hooks")]
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
    pub(super) session_promoted: bool,
    pub(super) trust_generation: u64,
    pub(super) trust_observer: Option<crate::executor::JoinHandle<()>>,
    pub(super) trust_recheck_task: Option<crate::executor::JoinHandle<()>>,
    /// One losslessly coalesced reducer demand that arrived while an
    /// authoritative query or its projection acknowledgement was unsettled.
    pub(super) trust_recheck_pending: bool,
    pub(super) current_session_status_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) current_session_status_request: Option<u64>,
    pub(super) secure_backup_ready: bool,
    pub(super) recovery_key_delivery_pending: bool,
    pub(super) secure_backup_inspection_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) secure_backup_monitor_task: Option<crate::executor::JoinHandle<()>>,
    pub(super) secure_backup_monitor_serial: u64,
    pub(super) secure_backup_inspection_pending: bool,
    pub(super) secure_backup_observer: Option<crate::executor::JoinHandle<()>>,
    pub(super) verification_method_discovery_task: Option<OwnedVerificationMethodDiscoveryTask>,
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
    #[cfg(feature = "test-hooks")]
    pub(super) residency_install_gap: Option<(
        oneshot::Sender<(
            Option<Arc<MatrixClientSession>>,
            Option<Arc<MatrixClientSession>>,
        )>,
        oneshot::Receiver<()>,
    )>,
    #[cfg(feature = "test-hooks")]
    pub(super) residency_teardown_gap: Option<(oneshot::Sender<bool>, oneshot::Receiver<()>)>,
    #[cfg(feature = "test-hooks")]
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
    /// Account-wide gate for `/rooms/{roomId}/messages` requests. Timeline
    /// pagination has priority over background search-history crawling.
    pub(super) account_work: crate::account_work::AccountWorkScheduler,
    pub(super) activity_resolution_task: Option<crate::executor::JoinHandle<()>>,
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
    /// Actor-private mapping from app-owned device ordinal to raw Matrix
    /// device id. Raw ids never enter reducer state or snapshots.
    pub(super) device_session_ordinals: BTreeMap<u64, String>,
    /// Pending UIA operations keyed by the flow id (original request id).
    /// Holds the data needed to retry a destructive action after the user
    /// supplies interactive auth. Secrets (password, UIA session) are held
    /// only inside this actor-private map, never in reducer state.
    pub(super) pending_uia_operations: BTreeMap<u64, PendingUiaOperation>,
    /// Opaque legacy UIAA continuation or local retry context. Raw SDK data
    /// never enters reducer state or diagnostics.
    pub(super) pending_device_cleanup: Option<PendingDeviceCleanup>,
    /// Pending OAuth authorization-code flow, keyed by originating request id.
    /// Holds SDK client, PKCE verifier, and CSRF validation data inside Rust.
    pub(super) pending_oidc_login: Option<(RequestId, PendingOidcFlow)>,
    #[cfg(test)]
    pub(super) oidc_completion_override: Option<MatrixClientSession>,
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
    /// Epoch attached to incoming verification messages from the active SDK client.
    pub(super) incoming_verification_session_generation: u64,
    /// SDK session-change observer for auth invalidation / soft logout.
    pub(super) session_change_observer: Option<SessionChangeObservation>,
    /// Optional profile/account-data hydration task for the active session.
    pub(super) account_hydration_task: Option<crate::executor::JoinHandle<()>>,
    /// Incremented whenever optional account hydration is invalidated.
    pub(super) account_hydration_generation: u64,
    /// Synthetic flow id sequence for SDK-originated verification requests.
    pub(super) next_incoming_verification_sequence: u64,
    /// Last `NotifySearchCrawlerRoomsAvailable` payload received before the
    /// `SearchActor` was spawned.  Replayed into the actor immediately after
    /// it is created so rooms that were already known to the reducer at
    /// session-restore time are not missed by the auto-start logic.
    pub(super) pending_crawler_notification:
        Option<(Vec<String>, koushi_state::SearchCrawlerSettings)>,
    /// Actor-owned avatar thumbnail cache: mxc_uri -> last resolved state.
    /// Mutated only from the actor loop; no shared lock needed.
    pub(super) avatar_cache: HashMap<String, AvatarThumbnailState>,
    /// In-flight fetches: mxc_uri -> waiting request_ids (single-flight dedup).
    /// The first `DownloadAvatarThumbnail` for a given mxc spawns a task and
    /// records its `request_id` here; subsequent ones for the same mxc while
    /// the task is running simply append their `request_id`. When `AvatarFetched`
    /// arrives every waiter receives `AvatarThumbnailDownloaded`.
    /// Entries are removed (and all waiters notified) when `AvatarFetched` arrives.
    pub(super) avatar_inflight: HashMap<String, Vec<RequestId>>,
    /// Semaphore bounding concurrent avatar downloads. Cloned into spawned
    /// fetch tasks; the actor holds one Arc so it can be replaced on session
    /// clear.
    pub(super) avatar_download_semaphore: Arc<Semaphore>,
    /// Owns all spawned avatar-fetch tasks. Aborted on session clear and
    /// shutdown (engineering-rules: every spawned task has an owner).
    pub(super) avatar_fetch_tasks: tokio::task::JoinSet<()>,
    /// Incremented by `abort_avatar_fetch_tasks` on every session clear /
    /// logout / switch / shutdown so that `AvatarFetched` completions that
    /// were already enqueued before the abort are detected and silently dropped
    /// instead of being accepted into the new (or absent) session's state.
    pub(super) avatar_session_generation: u64,
}

pub(super) fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
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
        // AppActor forwards every Room/Timeline/Sync command here via send().await;
        // sized so heavy sync does not block the AppActor's forwarding.
        let (tx, command_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let data_dir = store_actor.data_dir().to_path_buf();
        // Spawn RoomActor once at AccountActor creation. It starts with no
        // session and waits for RoomMessage::SyncStarted.
        let account_work = crate::account_work::AccountWorkScheduler::default();
        let room_actor = crate::room::RoomActor::spawn_with_account_work(
            action_tx.clone(),
            event_tx.clone(),
            sliding_sync_diagnostics.clone(),
            account_work.clone(),
        );
        let (navigation_projection, navigation_projection_rx) =
            NavigationProjectionIngress::channel();
        // Spawn TimelineManagerActor. It starts with no session; the session
        // is injected when a store-backed session is established.
        let timeline_manager = crate::timeline::TimelineManagerActor::spawn(
            action_tx.clone(),
            event_tx.clone(),
            Some(data_dir.clone()),
            account_work.clone(),
            Some(navigation_projection_rx),
        );
        #[cfg(feature = "test-hooks")]
        let residency_room_tx = room_actor.tx.clone();
        #[cfg(feature = "test-hooks")]
        let residency_room_operation_reached_count = room_actor.operation_test_reached_count();
        let actor = AccountActor {
            session: None,
            session_key_id: None,
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
            session_promoted: false,
            trust_generation: 0,
            trust_observer: None,
            trust_recheck_task: None,
            trust_recheck_pending: false,
            current_session_status_task: None,
            current_session_status_request: None,
            secure_backup_ready: false,
            recovery_key_delivery_pending: false,
            secure_backup_inspection_task: None,
            secure_backup_monitor_task: None,
            secure_backup_monitor_serial: 0,
            secure_backup_inspection_pending: false,
            secure_backup_observer: None,
            verification_method_discovery_task: None,
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
            #[cfg(feature = "test-hooks")]
            residency_install_gap: None,
            #[cfg(feature = "test-hooks")]
            residency_teardown_gap: None,
            #[cfg(feature = "test-hooks")]
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
            account_work,
            activity_resolution_task: None,
            data_dir,
            link_preview_policy: initial_link_preview_policy,
            send_read_receipts: initial_send_read_receipts,
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            identity_reset_flow_id: None,
            identity_reset_timeout_task: None,
            device_session_ordinals: BTreeMap::new(),
            pending_uia_operations: BTreeMap::new(),
            pending_device_cleanup: None,
            pending_oidc_login: None,
            #[cfg(test)]
            oidc_completion_override: None,
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
        AccountActorHandle {
            tx,
            navigation_projection,
            #[cfg(feature = "test-hooks")]
            residency_room_tx,
            #[cfg(feature = "test-hooks")]
            residency_room_operation_reached_count,
        }
    }

    async fn run(mut self) {
        #[cfg(any(test, feature = "test-hooks"))]
        let mut shutdown_ack: Option<oneshot::Sender<()>> = None;
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
                AccountMessage::ContinueSlidingSyncAdmission {
                    account_epoch,
                    request_id,
                    source,
                } => {
                    self.continue_sliding_sync_admission(account_epoch, request_id, source)
                        .await;
                }
                AccountMessage::RetrySlidingSyncCapabilityDiscovery {
                    account_epoch,
                    blocked_request_id,
                    request_id,
                } => {
                    self.start_sliding_sync_capability_retry(
                        account_epoch,
                        blocked_request_id,
                        request_id,
                    )
                    .await;
                }
                AccountMessage::ScheduleSlidingSyncCapabilityRevalidation { account_epoch } => {
                    if account_epoch == self.sliding_sync_account_epoch {
                        self.sliding_sync_revalidation_pending = Some(account_epoch);
                        if self.session_promoted {
                            self.start_sliding_sync_revalidation(account_epoch).await;
                        }
                    }
                }
                AccountMessage::SlidingSyncCapabilityDiscovered {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    self.finish_sliding_sync_capability_discovery(
                        account_epoch,
                        request_id,
                        result,
                    )
                    .await;
                }
                AccountMessage::SlidingSyncCapabilityRevalidationDiscovered {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    self.finish_sliding_sync_revalidation(account_epoch, request_id, result)
                        .await;
                }
                AccountMessage::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    self.settle_sliding_sync_revalidation(account_epoch, request_id, result)
                        .await;
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
                AccountMessage::ReadStatePolicyChanged { send_read_receipts } => {
                    self.send_read_receipts = send_read_receipts;
                    let _ = self
                        .timeline_manager
                        .set_read_state_policy(
                            self.read_persistence_session_generation,
                            send_read_receipts,
                        )
                        .await;
                }
                AccountMessage::TimelineCommandWithComposerFormatting {
                    command,
                    formatting_options,
                } => {
                    self.route_timeline_command_with_formatting_options(
                        command,
                        formatting_options,
                    )
                    .await;
                }
                AccountMessage::LeasedTimelineCommand {
                    command,
                    composer_permit,
                } => {
                    self.route_leased_timeline_command(command, composer_permit)
                        .await;
                }
                AccountMessage::LeasedTimelineCommandWithComposerFormatting {
                    command,
                    composer_permit,
                    formatting_options,
                } => {
                    self.route_leased_timeline_command_with_formatting_options(
                        command,
                        composer_permit,
                        formatting_options,
                    )
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
                    let backpressure = self.account_work.clone();
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
                    expected_account,
                    scheduled_id,
                    room_id,
                    thread_root_event_id,
                    body,
                    send_at_ms,
                    draft_revision,
                    composer_permit,
                } => {
                    self.handle_schedule_server_delayed_send(
                        request_id,
                        expected_account,
                        scheduled_id,
                        room_id,
                        thread_root_event_id,
                        body,
                        send_at_ms,
                        draft_revision,
                        composer_permit,
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
                AccountMessage::CurrentSessionStatusRefreshFinished {
                    request_id,
                    generation,
                    sync_state,
                    started_at,
                    result,
                } => {
                    self.finish_current_session_status_refresh(
                        request_id, generation, sync_state, started_at, result,
                    )
                    .await;
                }
                AccountMessage::CurrentDeviceTrustRecheckFinished { generation, result } => {
                    if generation != self.trust_generation {
                        continue;
                    }
                    let settled_stage = match &result {
                        Ok(koushi_state::CurrentDeviceTrustState::Verified) => {
                            "trust_recheck_finished_verified"
                        }
                        Ok(koushi_state::CurrentDeviceTrustState::Unverified) => {
                            "trust_recheck_finished_unverified"
                        }
                        Ok(koushi_state::CurrentDeviceTrustState::Unknown) => {
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
                    let trust = match result {
                        Ok(trust) => Some(trust),
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
                        self.handle_current_device_trust(generation, trust).await;
                    }
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
                        self.provisional_encryption_sync_ready = succeeded;
                        let trust = self
                            .session
                            .as_ref()
                            .map(|session| session.current_device_trust())
                            .unwrap_or(koushi_state::CurrentDeviceTrustState::Unknown);
                        if succeeded && should_discover_verification_methods(trust) {
                            self.discover_verification_methods(generation).await;
                        } else if succeeded
                            && trust == koushi_state::CurrentDeviceTrustState::Verified
                        {
                            self.handle_current_device_trust(generation, trust).await;
                        } else if trust == koushi_state::CurrentDeviceTrustState::Unknown {
                            self.request_authoritative_trust_recheck();
                        } else {
                            self.verification_method_discovery_failed = true;
                            self.send_actions(vec![AppAction::VerificationMethodDiscoveryFailed {
                                generation,
                                kind: koushi_state::VerificationGateFailureKind::Sdk,
                            }])
                            .await;
                        }
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
                        self.recheck_own_user_sas_after_sync().await;
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
                                self.send_actions(vec![AppAction::VerificationMethodsDiscovered(
                                    gate,
                                )])
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
                AccountMessage::RecoveryTrustSettlementTimedOut {
                    generation,
                    flow_id,
                    request_id,
                    trust,
                } => {
                    self.handle_recovery_trust_settlement_timed_out(
                        generation, flow_id, request_id, trust,
                    )
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
                AccountMessage::InspectSecureBackup => {
                    self.start_secure_backup_inspection();
                }
                AccountMessage::SecureBackupInspectionFinished { generation, result } => {
                    self.finish_secure_backup_inspection(generation, result)
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
                    self.handle_secure_backup_state_changed(generation, state)
                        .await;
                }
                AccountMessage::RejectProvisionalSession { request_id } => {
                    self.perform_logout(request_id, true, false).await;
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
                AccountMessage::InspectPendingDeviceCleanup { response } => {
                    let _ = response.send(self.pending_device_cleanup.is_some());
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
                    self.handle_current_device_trust(self.trust_generation, trust)
                        .await;
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestConfigureInstallGap {
                    reached,
                    release,
                    configured,
                } => {
                    self.residency_install_gap = Some((reached, release));
                    let _ = configured.send(());
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestConfigureTeardownGap {
                    reached,
                    release,
                    configured,
                } => {
                    self.residency_teardown_gap = Some((reached, release));
                    let _ = configured.send(());
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestInstallSession { session, completed } => {
                    self.install_residency_test_session(session).await;
                    let _ = completed.send(());
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestRoomCommand { command, accepted } => {
                    self.route_room_command(command).await;
                    let _ = accepted.send(());
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestConfigureRoomOperation {
                    control,
                    configured,
                } => {
                    let _ = configured
                        .send(self.room_actor.install_room_operation_test_control(control));
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestTimelineSnapshot { response } => {
                    let _ = self
                        .timeline_manager
                        .residency_snapshot_for_testing(response)
                        .await;
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestTimelineGateSnapshot { response } => {
                    let _ =
                        response.send(self.timeline_manager.residency_gate_snapshot_for_testing());
                }
                #[cfg(feature = "test-hooks")]
                AccountMessage::ResidencyTestShutdown { acknowledged } => {
                    shutdown_ack = Some(acknowledged);
                    break;
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
                #[cfg(test)]
                AccountMessage::ConfigureDeviceCleanupResults { results } => {
                    self.device_cleanup_results = results.into();
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
                AccountMessage::SessionInvalidated { reason } => {
                    self.handle_session_invalidated(reason).await;
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
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(acknowledged) = shutdown_ack {
            let _ = acknowledged.send(());
        }
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
                platform,
            } => {
                self.handle_complete_oidc_login(request_id, callback_url, platform)
                    .await;
            }
            AccountCommand::LoginPassword {
                request_id,
                request,
                platform,
            } => {
                self.handle_login_password(request_id, request, platform)
                    .await;
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
            AccountCommand::RetrySlidingSyncCapability { request_id } => {
                self.handle_retry_sliding_sync_capability(request_id).await;
            }
            AccountCommand::QuerySavedSessions { request_id } => {
                self.handle_query_saved_sessions(request_id).await;
            }
            AccountCommand::QueryDevices { request_id } => {
                self.handle_query_devices(request_id).await;
            }
            AccountCommand::RefreshCurrentSessionStatus { .. } => {
                // The AppActor routes the reducer-owned refresh effect with
                // the authoritative sync projection.
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
            AccountCommand::RecoverSecureBackup {
                request_id,
                request,
            } => {
                self.handle_recover_secure_backup(request_id, request).await;
            }
            AccountCommand::RetrySecureBackupInspection { .. } => {
                self.start_secure_backup_inspection();
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
            AccountCommand::StartDeviceCleanup { request_id } => {
                self.handle_start_device_cleanup(request_id).await;
            }
            AccountCommand::SubmitDeviceCleanupUia {
                request_id,
                flow_id,
                password,
            } => {
                self.handle_submit_device_cleanup_uia(request_id, flow_id, password)
                    .await;
            }
            AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id } => {
                self.handle_erase_device_cleanup_local_data_anyway(request_id)
                    .await;
            }
            AccountCommand::Logout { request_id } => {
                self.handle_logout(request_id).await;
            }
            AccountCommand::ChangeHomeserver { request_id } => {
                self.handle_change_homeserver(request_id).await;
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
                self.request_authoritative_trust_recheck();
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
                    self.request_authoritative_trust_recheck();
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

    pub(super) fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
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

    use crate::ids::{RequestId, RuntimeConnectionId};

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

    #[test]
    fn account_actor_reducer_actions_use_reliable_delivery() {
        let production_sources = [
            include_str!("account_management.rs"),
            include_str!("actor.rs"),
            include_str!("local_data_cleanup.rs"),
            include_str!("profile.rs"),
            include_str!("recovery_backup.rs"),
            include_str!("routing.rs"),
            include_str!("runtime_children.rs"),
            include_str!("scheduled_send.rs"),
            include_str!("session_lifecycle.rs"),
            include_str!("sliding_sync.rs"),
            include_str!("trust_gate.rs"),
            include_str!("verification.rs"),
        ];
        let production_source = |source: &'static str| {
            source
                .split("\n#[cfg(test)]\nmod tests {")
                .next()
                .unwrap_or(source)
        };
        let send_actions_body = crate::account::test_source::item_body(
            include_str!("actor.rs"),
            "async fn send_actions",
        );

        assert!(
            send_actions_body.contains("self.action_tx.send(actions).await"),
            "AccountActor reducer actions must await reliable delivery"
        );
        assert!(
            production_sources
                .iter()
                .all(|source| !production_source(source).contains("self.reduce(")),
            "AccountActor command-result reducer actions must not use the lossy reduce helper"
        );
        assert!(
            production_sources.iter().all(|source| {
                !production_source(source).contains("action_tx.try_send(actions)")
            }),
            "AccountActor reducer actions must not be dropped through try_send"
        );
    }
}
