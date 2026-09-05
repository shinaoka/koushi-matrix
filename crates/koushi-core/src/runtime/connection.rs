use super::{CoreCommandEnvelope, CoreRuntime};
use crate::command_policy::{CoreCommandPolicy, native_artifact_for_command};
use crate::composer_draft_lifecycle::{
    ComposerDraftCommandPermit, ComposerDraftLeaseAdmission, ComposerDraftLeaseAdmissionFailure,
    ComposerDraftLeaseFailure, ComposerDraftLeaseId, ComposerDraftLeaseRegistry,
    ComposerDraftScope, ComposerRendererGeneration,
};
use crate::event_projection::{
    project_room_event_display_labels, project_timeline_event_display_labels,
};
use crate::media_staging::MediaStagingService;
use crate::native_artifact::{NativeArtifactError, NativeArtifactKind, NativeArtifactPort};
use koushi_protocol::command::{
    AppCommand, CoreCommand, EventNavigationMissingTargetPolicy, RoomCommand,
};
#[cfg(test)]
use koushi_protocol::event::IntentOutcome;
use koushi_protocol::event::{CoreEvent, IntentNoOpReason};
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_protocol::state_update::{
    AppStateSnapshot, CoreCommandAdmission, VersionedAppStateSnapshot,
};
use koushi_state::ComposerDraftRevision;
use koushi_state::{EventNavigationFailureKind, EventNavigationSource, EventNavigationState};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CommandSubmitError {
    #[error("core runtime is closed")]
    RuntimeClosed,
    #[error("request id does not belong to this connection")]
    InvalidRequestId,
    #[error("composer draft command requires lease admission")]
    ComposerLeaseRequired,
    #[error("command does not carry a composer draft revision")]
    ComposerLeaseNotRequired,
    #[error("composer draft lease admission failed")]
    ComposerLease(ComposerDraftLeaseFailure),
    #[error("native artifact registration failed")]
    NativeArtifact(NativeArtifactError),
}

/// Typed terminal failures returned by [`CoreConnection::select_room_and_wait`].
///
/// A matching `Committed` or benign no-op lifecycle event is only progress;
/// selection succeeds once the requested room is visible in the latest versioned
/// watch snapshot. Other requests and lagged broadcast events are ignored or
/// recovered from that snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EventNavigationError {
    #[error("event navigation command could not be submitted: {0}")]
    CommandSubmit(#[source] CommandSubmitError),
    #[error("event navigation command was rejected")]
    Rejected,
    #[error("event navigation failed: {0:?}")]
    Failed(EventNavigationFailureKind),
    #[error("core event stream closed")]
    EventStreamClosed,
    #[error("event navigation timed out")]
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SelectRoomError {
    #[error("room selection command could not be submitted: {0}")]
    CommandSubmit(#[source] CommandSubmitError),
    #[error("room selection requires a ready session")]
    SessionNotReady,
    #[error("room is not present in the current state")]
    RoomNotInState,
    #[error("room selection failed without a state change: {0:?}")]
    FailedNoOp(IntentNoOpReason),
    #[error("room selection operation failed: {0:?}")]
    OperationFailed(koushi_protocol::failure::CoreFailure),
    #[error("core event stream closed")]
    EventStreamClosed,
    #[error("room selection timed out")]
    Timeout,
}

/// Surfaced when a consumer fell behind the bounded event queue. The
/// consumer must resync from the latest snapshot and (in later phases) the
/// per-timeline resync events; intermediate discrete events were dropped
/// for this consumer only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventStreamLag {
    pub skipped: u64,
}

/// One attached consumer: allocates request ids, submits commands, and
/// observes the shared event stream plus the latest snapshot.
pub struct CoreConnection {
    connection_id: RuntimeConnectionId,
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    pub(super) native_artifacts: Arc<dyn NativeArtifactPort>,
    pub(super) media_staging: Arc<MediaStagingService>,
    pub(super) event_rx: broadcast::Receiver<CoreEvent>,
    pub(super) snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    next_sequence: AtomicU64,
}

/// Lightweight command submitter that can be cloned without cloning event or
/// snapshot receivers.
#[derive(Clone)]
pub struct CoreCommandHandle {
    connection_id: RuntimeConnectionId,
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    native_artifacts: Arc<dyn NativeArtifactPort>,
}

struct NativeArtifactRegistrationGuard {
    port: Arc<dyn NativeArtifactPort>,
    request_id: RequestId,
    kind: NativeArtifactKind,
    armed: bool,
}

impl NativeArtifactRegistrationGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for NativeArtifactRegistrationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.port.unregister(self.request_id, self.kind);
        }
    }
}

impl CoreRuntime {
    /// Attach a consumer. Returns its connection handle; the handle's
    /// `RuntimeConnectionId` is the only id its commands may carry.
    pub fn attach(&self) -> CoreConnection {
        CoreConnection {
            connection_id: RuntimeConnectionId(
                self.next_connection_id.fetch_add(1, Ordering::Relaxed),
            ),
            command_tx: self.command_tx.clone(),
            composer_draft_leases: Arc::clone(&self.composer_draft_leases),
            native_artifacts: Arc::clone(&self.native_artifacts),
            media_staging: Arc::clone(&self.media_staging),
            event_rx: self.event_tx.subscribe(),
            snapshot_rx: self.snapshot_rx.clone(),
            next_sequence: AtomicU64::new(1),
        }
    }
}

impl CoreCommandHandle {
    /// Submit a command without a composer lease. Fails locally — before
    /// routing and before any `CoreEvent` is published — if the request id
    /// belongs to another connection or the command carries a composer
    /// revision and therefore requires [`Self::command_with_composer_lease`].
    pub async fn command(&self, command: CoreCommand) -> Result<(), CommandSubmitError> {
        self.validate_request_id(&command)?;
        if command.composer_draft_scope().is_some() {
            return Err(CommandSubmitError::ComposerLeaseRequired);
        }
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command,
                composer_permit: None,
                admission: None,
            })
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    /// Submit a command and wait until AppActor has handled it and published
    /// the synchronous state it owns. This is admission, not terminal outcome.
    pub async fn command_with_admission(
        &self,
        command: CoreCommand,
    ) -> Result<CoreCommandAdmission, CommandSubmitError> {
        self.validate_request_id(&command)?;
        if command.composer_draft_scope().is_some() {
            return Err(CommandSubmitError::ComposerLeaseRequired);
        }
        let (admission_tx, admission_rx) = oneshot::channel();
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command,
                composer_permit: None,
                admission: Some(admission_tx),
            })
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)?;
        admission_rx
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    /// Register one native path and transfer its ownership atomically with
    /// command enqueue. Cancellation before enqueue removes the registration;
    /// after enqueue, Core owns consumption or rejection cleanup.
    pub async fn command_with_native_artifact_and_admission(
        &self,
        command: CoreCommand,
        kind: NativeArtifactKind,
        path: std::path::PathBuf,
    ) -> Result<CoreCommandAdmission, CommandSubmitError> {
        self.validate_request_id(&command)?;
        if command.composer_draft_scope().is_some() {
            return Err(CommandSubmitError::ComposerLeaseRequired);
        }
        let request_id = command.request_id();
        if native_artifact_for_command(&command) != Some((request_id, kind)) {
            return Err(CommandSubmitError::NativeArtifact(
                NativeArtifactError::Missing,
            ));
        }
        self.native_artifacts
            .register(request_id, kind, path)
            .map_err(CommandSubmitError::NativeArtifact)?;
        let mut registration = NativeArtifactRegistrationGuard {
            port: Arc::clone(&self.native_artifacts),
            request_id,
            kind,
            armed: true,
        };
        let (admission_tx, admission_rx) = oneshot::channel();
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command,
                composer_permit: None,
                admission: Some(admission_tx),
            })
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)?;
        registration.disarm();
        admission_rx
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    pub fn begin_composer_draft_renderer_generation(
        &self,
    ) -> Result<ComposerRendererGeneration, ComposerDraftLeaseFailure> {
        self.composer_draft_leases.begin_renderer_generation()
    }

    pub fn acquire_composer_draft_lease(
        &self,
        generation: ComposerRendererGeneration,
        scope: ComposerDraftScope,
    ) -> Result<ComposerDraftLeaseId, ComposerDraftLeaseFailure> {
        self.composer_draft_leases.acquire(generation, scope)
    }

    pub fn release_composer_draft_lease(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
    ) -> Result<(), ComposerDraftLeaseFailure> {
        self.composer_draft_leases.release(generation, lease_id)
    }

    pub fn acquire_composer_draft_command_permit(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        scope: &ComposerDraftScope,
    ) -> Result<ComposerDraftCommandPermit, ComposerDraftLeaseFailure> {
        self.composer_draft_leases
            .try_command_permit(generation, lease_id, scope)
    }

    pub async fn command_with_composer_lease(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        command: CoreCommand,
    ) -> Result<(), CommandSubmitError> {
        let envelope = self.admit_composer_command(generation, lease_id, command)?;
        self.command_tx
            .send(envelope)
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    pub async fn command_with_composer_lease_and_admission(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        command: CoreCommand,
    ) -> Result<CoreCommandAdmission, CommandSubmitError> {
        let envelope = self.admit_composer_command(generation, lease_id, command)?;
        let (admission_tx, admission_rx) = oneshot::channel();
        let (command, composer_permit) = match envelope {
            CoreCommandEnvelope::Public {
                command,
                composer_permit,
                admission: _,
            } => (command, composer_permit),
            #[cfg(any(test, feature = "test-hooks"))]
            CoreCommandEnvelope::Qa(_) => {
                unreachable!("composer admission creates a public command")
            }
        };
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command,
                composer_permit,
                admission: Some(admission_tx),
            })
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)?;
        admission_rx
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn command_with_composer_lease_after_admission(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        command: CoreCommand,
        admitted: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    ) -> Result<(), CommandSubmitError> {
        let envelope = self.admit_composer_command(generation, lease_id, command)?;
        let _ = admitted.send(());
        let _ = release.await;
        self.command_tx
            .send(envelope)
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    fn validate_request_id(&self, command: &CoreCommand) -> Result<(), CommandSubmitError> {
        if command.request_id().connection_id != self.connection_id {
            return Err(CommandSubmitError::InvalidRequestId);
        }
        Ok(())
    }

    fn admit_composer_command(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        command: CoreCommand,
    ) -> Result<CoreCommandEnvelope, CommandSubmitError> {
        self.validate_request_id(&command)?;
        let scope = command
            .composer_draft_scope()
            .ok_or(CommandSubmitError::ComposerLeaseNotRequired)?;
        let composer_permit = self
            .composer_draft_leases
            .try_command_permit(generation, lease_id, &scope)
            .map_err(CommandSubmitError::ComposerLease)?;
        Ok(CoreCommandEnvelope::Public {
            command,
            composer_permit: Some(composer_permit),
            admission: None,
        })
    }
}

#[cfg(any(test, feature = "test-hooks"))]
#[doc(hidden)]
pub struct CoreConnectionTestControl {
    command_rx: mpsc::Receiver<CoreCommandEnvelope>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_tx: watch::Sender<VersionedAppStateSnapshot>,
}

#[cfg(any(test, feature = "test-hooks"))]
impl CoreConnectionTestControl {
    #[doc(hidden)]
    pub async fn recv_command(&mut self) -> Option<CoreCommand> {
        self.command_rx.recv().await.map(|envelope| match envelope {
            CoreCommandEnvelope::Public {
                command, admission, ..
            } => {
                if let Some(admission) = admission {
                    let admitted_generation = self.snapshot_tx.borrow().generation;
                    let _ = admission.send(CoreCommandAdmission {
                        admitted_generation,
                    });
                }
                command
            }
            #[cfg(any(test, feature = "test-hooks"))]
            CoreCommandEnvelope::Qa(_) => unreachable!("test control received QA command"),
        })
    }

    #[doc(hidden)]
    pub fn send_event(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    #[doc(hidden)]
    pub fn send_snapshot(&self, snapshot: VersionedAppStateSnapshot) {
        let _ = self.snapshot_tx.send(snapshot);
    }
}

impl CoreConnection {
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub fn new_for_testing(event_capacity: usize) -> (Self, CoreConnectionTestControl) {
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = broadcast::channel(event_capacity);
        let (snapshot_tx, snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
            generation: 0,
            state: koushi_state::AppState::default(),
        });
        (
            Self {
                connection_id: RuntimeConnectionId(41),
                command_tx,
                composer_draft_leases: Arc::new(ComposerDraftLeaseRegistry::new()),
                native_artifacts: Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
                media_staging: Arc::new(MediaStagingService::new(Arc::new(
                    crate::media_preparation::MediaPreparationService::default(),
                ))),
                event_rx,
                snapshot_rx,
                next_sequence: AtomicU64::new(1),
            },
            CoreConnectionTestControl {
                command_rx,
                event_tx,
                snapshot_tx,
            },
        )
    }

    pub fn connection_id(&self) -> RuntimeConnectionId {
        self.connection_id
    }

    pub fn register_native_artifact(
        &self,
        request_id: RequestId,
        kind: NativeArtifactKind,
        path: std::path::PathBuf,
    ) -> Result<(), NativeArtifactError> {
        self.native_artifacts.register(request_id, kind, path)
    }

    pub fn unregister_native_artifact(&self, request_id: RequestId, kind: NativeArtifactKind) {
        self.native_artifacts.unregister(request_id, kind);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn qa_set_local_device_blacklisted(
        &self,
        target: koushi_state::VerificationTarget,
        room_id: String,
    ) -> Result<(), ()> {
        let request_id = self.next_request_id();
        let (acknowledged, result) = oneshot::channel();
        self.command_tx
            .send(super::CoreCommandEnvelope::Qa(
                super::CoreQaCommand::SetLocalDeviceBlacklisted {
                    request_id,
                    target,
                    room_id,
                    acknowledged,
                },
            ))
            .await
            .map_err(|_| ())?;
        result.await.map_err(|_| ())?
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn qa_refresh_device_keys_and_assert_known(
        &self,
        target: koushi_state::VerificationTarget,
    ) -> Result<(), ()> {
        let request_id = self.next_request_id();
        let (acknowledged, result) = oneshot::channel();
        self.command_tx
            .send(super::CoreCommandEnvelope::Qa(
                super::CoreQaCommand::RefreshDeviceKeysAndAssertKnown {
                    request_id,
                    target,
                    acknowledged,
                },
            ))
            .await
            .map_err(|_| ())?;
        result.await.map_err(|_| ())?
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn qa_assert_inbound_sessions_start_at_zero(
        &self,
        room_id: String,
    ) -> Result<usize, ()> {
        let request_id = self.next_request_id();
        let (acknowledged, result) = oneshot::channel();
        self.command_tx
            .send(super::CoreCommandEnvelope::Qa(
                super::CoreQaCommand::AssertInboundSessionsStartAtZero {
                    request_id,
                    room_id,
                    acknowledged,
                },
            ))
            .await
            .map_err(|_| ())?;
        result.await.map_err(|_| ())?
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn sync_once_for_qa(&self) -> Result<(), CommandSubmitError> {
        let request_id = self.next_request_id();
        self.command_tx
            .send(super::CoreCommandEnvelope::Qa(
                super::CoreQaCommand::SyncOnce { request_id },
            ))
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    /// Clone a lightweight command submitter for callers that must not hold
    /// the full connection guard while awaiting bounded channel capacity.
    pub fn command_handle(&self) -> CoreCommandHandle {
        CoreCommandHandle {
            connection_id: self.connection_id,
            command_tx: self.command_tx.clone(),
            composer_draft_leases: Arc::clone(&self.composer_draft_leases),
            native_artifacts: Arc::clone(&self.native_artifacts),
        }
    }

    /// Allocate the next request id for this connection. Request ids are
    /// allocated here, never hand-built by callers.
    pub fn next_request_id(&self) -> RequestId {
        RequestId {
            connection_id: self.connection_id,
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Stage bytes through the Core-owned media preparation service.
    pub async fn stage_upload_bytes(
        &mut self,
        target: koushi_state::ComposerTarget,
        items: Vec<crate::media_preparation::StageUploadBytesInput>,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service.stage_upload_bytes(self, target, items).await
    }

    pub async fn select_staged_upload_output(
        &mut self,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        selection: koushi_state::StagedUploadOutputSelection,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service
            .select_staged_upload_output(self, target, staged_id, selection)
            .await
    }

    pub async fn retry_staged_upload_preparation(
        &mut self,
        target: koushi_state::ComposerTarget,
        staged_id: String,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service
            .retry_staged_upload_preparation(self, target, staged_id)
            .await
    }

    pub async fn update_staged_upload_caption(
        &mut self,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        caption: Option<koushi_state::ComposerDocument>,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service
            .update_caption(self, target, staged_id, caption)
            .await
    }

    pub async fn update_staged_upload_compression(
        &mut self,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        compression_choice: koushi_state::StagedUploadCompressionChoice,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service
            .update_compression(self, target, staged_id, compression_choice)
            .await
    }

    pub async fn use_original_staged_upload(
        &mut self,
        target: koushi_state::ComposerTarget,
        staged_id: String,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service.use_original(self, target, staged_id).await
    }

    pub async fn clear_upload_staging(
        &mut self,
        target: koushi_state::ComposerTarget,
    ) -> Result<VersionedAppStateSnapshot, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service.clear(self, target).await
    }

    pub async fn prepared_upload_preview(
        &mut self,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        variant_id: String,
    ) -> Result<Vec<u8>, crate::media_staging::MediaStagingError> {
        let service = Arc::clone(&self.media_staging);
        service
            .prepared_upload_preview(self, target, staged_id, variant_id)
            .await
    }

    pub async fn send_prepared_uploads(
        &mut self,
        expected_account: koushi_protocol::SessionKeyId,
        generation: crate::composer_draft_lifecycle::ComposerRendererGeneration,
        lease: crate::composer_draft_lifecycle::ComposerDraftLeaseId,
        target: koushi_state::ComposerTarget,
        draft_revision: koushi_state::ComposerDraftRevision,
    ) -> Result<
        crate::media_staging::PreparedUploadSendResult,
        crate::media_staging::PreparedUploadSendError,
    > {
        let service = Arc::clone(&self.media_staging);
        service
            .send_prepared_uploads(
                self,
                expected_account,
                generation,
                lease,
                target,
                draft_revision,
            )
            .await
    }

    /// Submit a command without a composer lease. Revision-bearing composer
    /// commands fail closed and must use [`Self::command_with_composer_lease`].
    pub async fn command(&self, command: CoreCommand) -> Result<(), CommandSubmitError> {
        self.command_handle().command(command).await
    }

    pub fn begin_composer_draft_renderer_generation(
        &self,
    ) -> Result<ComposerRendererGeneration, ComposerDraftLeaseFailure> {
        self.command_handle()
            .begin_composer_draft_renderer_generation()
    }

    pub fn acquire_composer_draft_lease(
        &self,
        generation: ComposerRendererGeneration,
        scope: ComposerDraftScope,
    ) -> Result<ComposerDraftLeaseId, ComposerDraftLeaseFailure> {
        self.command_handle()
            .acquire_composer_draft_lease(generation, scope)
    }

    pub fn acquire_composer_draft_lease_for_active_target(
        &self,
        expected_account: koushi_protocol::SessionKeyId,
        generation: ComposerRendererGeneration,
        target: koushi_state::ComposerTarget,
    ) -> Result<ComposerDraftLeaseAdmission, ComposerDraftLeaseAdmissionFailure> {
        let snapshot = self.snapshot();
        validate_active_composer_scope(&snapshot, &expected_account, &target)?;
        let lease_id = self
            .composer_draft_leases
            .acquire(
                generation,
                ComposerDraftScope {
                    account: expected_account,
                    target: target.clone(),
                },
            )
            .map_err(ComposerDraftLeaseAdmissionFailure::Registry)?;
        Ok(ComposerDraftLeaseAdmission {
            lease_id,
            revision: composer_draft_revision(&snapshot, &target),
            last_accepted_clear_revision: composer_draft_last_accepted_clear_revision(
                &snapshot, &target,
            ),
            has_authoritative_content: composer_draft_has_content(&snapshot, &target),
        })
    }

    pub fn acquire_composer_draft_command_permit_for_active_target(
        &self,
        expected_account: koushi_protocol::SessionKeyId,
        target: koushi_state::ComposerTarget,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
    ) -> Result<ComposerDraftCommandPermit, ComposerDraftLeaseAdmissionFailure> {
        let snapshot = self.snapshot();
        validate_active_composer_scope(&snapshot, &expected_account, &target)?;
        self.composer_draft_leases
            .try_command_permit(
                generation,
                lease_id,
                &ComposerDraftScope {
                    account: expected_account,
                    target,
                },
            )
            .map_err(ComposerDraftLeaseAdmissionFailure::Registry)
    }

    pub fn release_composer_draft_lease(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
    ) -> Result<(), ComposerDraftLeaseFailure> {
        self.command_handle()
            .release_composer_draft_lease(generation, lease_id)
    }

    pub fn acquire_composer_draft_command_permit(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        scope: &ComposerDraftScope,
    ) -> Result<ComposerDraftCommandPermit, ComposerDraftLeaseFailure> {
        self.command_handle()
            .acquire_composer_draft_command_permit(generation, lease_id, scope)
    }

    pub async fn command_with_admission(
        &self,
        command: CoreCommand,
    ) -> Result<CoreCommandAdmission, CommandSubmitError> {
        self.command_handle().command_with_admission(command).await
    }

    pub async fn command_with_composer_lease(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        command: CoreCommand,
    ) -> Result<(), CommandSubmitError> {
        self.command_handle()
            .command_with_composer_lease(generation, lease_id, command)
            .await
    }

    pub async fn command_with_composer_lease_and_admission(
        &self,
        generation: ComposerRendererGeneration,
        lease_id: ComposerDraftLeaseId,
        command: CoreCommand,
    ) -> Result<CoreCommandAdmission, CommandSubmitError> {
        self.command_handle()
            .command_with_composer_lease_and_admission(generation, lease_id, command)
            .await
    }

    /// Receive the next event. On lag, intermediate events were dropped for
    /// this consumer; resync from [`Self::snapshot`].
    pub async fn recv_event(&mut self) -> Result<CoreEvent, EventStreamLag> {
        loop {
            match self.event_rx.recv().await {
                Ok(event) => return Ok(self.project_event_for_consumer(event)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    return Err(EventStreamLag { skipped });
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // Runtime shut down; surface as lag so callers resync and
                    // observe the final snapshot.
                    return Err(EventStreamLag { skipped: 0 });
                }
            }
        }
    }

    pub(super) fn project_event_for_consumer(&self, mut event: CoreEvent) -> CoreEvent {
        match &mut event {
            CoreEvent::Timeline(timeline_event) => {
                let snapshot = self.snapshot_rx.borrow().state.clone();
                project_timeline_event_display_labels(timeline_event, &snapshot);
            }
            CoreEvent::Room(room_event) => {
                let snapshot = self.snapshot_rx.borrow().state.clone();
                project_room_event_display_labels(room_event, &snapshot);
            }
            CoreEvent::StateDelta(_)
            | CoreEvent::Account(_)
            | CoreEvent::Sync(_)
            | CoreEvent::LiveSignals(_)
            | CoreEvent::Search(_)
            | CoreEvent::E2eeTrust(_)
            | CoreEvent::Activity(_)
            | CoreEvent::LocalEncryption(_)
            | CoreEvent::NativeAttention(_)
            | CoreEvent::CjkTextPolicy(_)
            | CoreEvent::ThreadsList(_)
            | CoreEvent::OperationFailed { .. }
            | CoreEvent::IntentLifecycle { .. } => {}
        }
        event
    }

    /// Latest state snapshot (latest-wins watch semantics).
    pub fn snapshot(&self) -> AppStateSnapshot {
        self.snapshot_rx.borrow().state.clone()
    }

    /// Latest state snapshot with the generation used by `StateDelta`.
    pub fn versioned_snapshot(&self) -> VersionedAppStateSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    /// Wait for the next latest-wins snapshot publication.
    ///
    /// The returned generation may equal the current generation when Core-only
    /// state outside the desktop `StateDelta` contract changes.
    pub async fn next_versioned_snapshot(&mut self) -> Option<VersionedAppStateSnapshot> {
        self.snapshot_rx.changed().await.ok()?;
        Some(self.snapshot_rx.borrow_and_update().clone())
    }

    /// Select `room_id` and wait until the latest versioned watch snapshot names
    /// it as the active room. The typed outcome service owns the event/snapshot
    /// settlement; this method preserves the historical error surface.
    pub async fn navigate_to_event_and_wait(
        &mut self,
        room_id: String,
        event_id: String,
        source: EventNavigationSource,
        missing_target_policy: EventNavigationMissingTargetPolicy,
        timeout: Duration,
    ) -> Result<VersionedAppStateSnapshot, EventNavigationError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let baseline = self.snapshot().navigation.event_navigation.generation();
        let generation = baseline
            .checked_add(1)
            .ok_or(EventNavigationError::Rejected)?;
        let request_id = self.next_request_id();
        tokio::time::timeout_at(
            deadline,
            self.command(CoreCommand::App(AppCommand::NavigateToEvent {
                request_id,
                room_id,
                event_id,
                source,
                missing_target_policy,
            })),
        )
        .await
        .map_err(|_| EventNavigationError::Timeout)?
        .map_err(EventNavigationError::CommandSubmit)?;

        let terminal = |snapshot: &VersionedAppStateSnapshot| {
            if snapshot.state.navigation.event_navigation.generation() > generation {
                return Some(Ok(snapshot.clone()));
            }
            match snapshot.state.navigation.event_navigation {
                EventNavigationState::Anchored {
                    generation: current,
                    ..
                }
                | EventNavigationState::LiveFallback {
                    generation: current,
                    ..
                } if current == generation => Some(Ok(snapshot.clone())),
                EventNavigationState::Failed {
                    generation: current,
                    failure_kind,
                    ..
                } if current == generation => Some(Err(EventNavigationError::Failed(failure_kind))),
                _ => None,
            }
        };

        if let Some(result) = terminal(&self.versioned_snapshot()) {
            return result;
        }
        loop {
            if let Some(result) = terminal(&self.versioned_snapshot()) {
                return result;
            }
            match tokio::time::timeout_at(deadline, self.snapshot_rx.changed()).await {
                Ok(Ok(())) => {
                    let _ = self.snapshot_rx.borrow_and_update();
                }
                Ok(Err(_)) => {
                    return terminal(&self.versioned_snapshot())
                        .unwrap_or(Err(EventNavigationError::EventStreamClosed));
                }
                Err(_) => {
                    return terminal(&self.versioned_snapshot())
                        .unwrap_or(Err(EventNavigationError::Timeout));
                }
            }
        }
    }

    pub async fn select_room_and_wait(
        &mut self,
        room_id: String,
        timeout: Duration,
    ) -> Result<VersionedAppStateSnapshot, SelectRoomError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let baseline_generation = self.versioned_snapshot().generation;
        let request_id = self.next_request_id();
        tokio::time::timeout_at(
            deadline,
            self.command(CoreCommand::Room(RoomCommand::SelectRoom {
                request_id,
                room_id: room_id.clone(),
            })),
        )
        .await
        .map_err(|_| SelectRoomError::Timeout)?
        .map_err(SelectRoomError::CommandSubmit)?;

        match self
            .wait_for_request_outcome(
                super::request_outcome::OutcomeCorrelation::Request(request_id),
                super::request_outcome::RequestOutcomeExpectation::RoomSelected {
                    request_id,
                    room_id,
                    account_key: None,
                    allow_initial: true,
                },
                baseline_generation,
                deadline,
            )
            .await
        {
            Ok(super::request_outcome::RequestOutcome::RoomSelected { snapshot }) => Ok(snapshot),
            Ok(_) => Err(SelectRoomError::Timeout),
            Err(super::request_outcome::RequestOutcomeError::OperationFailed { failure }) => {
                Err(SelectRoomError::OperationFailed(failure))
            }
            Err(super::request_outcome::RequestOutcomeError::FailedNoOp { reason }) => {
                Err(match reason {
                    IntentNoOpReason::SessionNotReady => SelectRoomError::SessionNotReady,
                    IntentNoOpReason::RoomNotInState => SelectRoomError::RoomNotInState,
                    reason => SelectRoomError::FailedNoOp(reason),
                })
            }
            Err(super::request_outcome::RequestOutcomeError::Disconnected) => {
                Err(SelectRoomError::EventStreamClosed)
            }
            Err(super::request_outcome::RequestOutcomeError::TimedOut)
            | Err(super::request_outcome::RequestOutcomeError::Lagged)
            | Err(super::request_outcome::RequestOutcomeError::InvalidOutcome) => {
                Err(SelectRoomError::Timeout)
            }
        }
    }
}

fn validate_active_composer_scope(
    snapshot: &AppStateSnapshot,
    expected_account: &koushi_protocol::SessionKeyId,
    target: &koushi_state::ComposerTarget,
) -> Result<(), ComposerDraftLeaseAdmissionFailure> {
    let koushi_state::SessionState::Ready(info) = &snapshot.session else {
        return Err(ComposerDraftLeaseAdmissionFailure::SessionNotReady);
    };
    if &crate::store::session_key_id_from_info(info) != expected_account {
        return Err(ComposerDraftLeaseAdmissionFailure::AccountMismatch);
    }
    let active = match target {
        koushi_state::ComposerTarget::Main { room_id } => {
            snapshot.timeline.room_id.as_deref() == Some(room_id)
        }
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => matches!(
            &snapshot.thread,
            koushi_state::ThreadPaneState::Open {
                room_id: active_room_id,
                root_event_id: active_root_event_id,
                ..
            } if active_room_id == room_id && active_root_event_id == root_event_id
        ),
    };
    active
        .then_some(())
        .ok_or(ComposerDraftLeaseAdmissionFailure::TargetInactive)
}

fn composer_draft_revision(
    state: &AppStateSnapshot,
    target: &koushi_state::ComposerTarget,
) -> ComposerDraftRevision {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => {
            state.composer_drafts.room_revision(room_id)
        }
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_revision(room_id, root_event_id),
    }
}

fn composer_draft_last_accepted_clear_revision(
    state: &AppStateSnapshot,
    target: &koushi_state::ComposerTarget,
) -> ComposerDraftRevision {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => state
            .composer_drafts
            .room_last_accepted_clear_revisions
            .get(room_id)
            .copied()
            .unwrap_or_default(),
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_last_accepted_clear_revisions
            .get(room_id)
            .and_then(|threads| threads.get(root_event_id))
            .copied()
            .unwrap_or_default(),
    }
}

fn composer_draft_has_content(
    state: &AppStateSnapshot,
    target: &koushi_state::ComposerTarget,
) -> bool {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => state
            .composer_drafts
            .rooms
            .get(room_id)
            .is_some_and(|draft| !draft.is_empty()),
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .threads
            .get(room_id)
            .and_then(|threads| threads.get(root_event_id))
            .is_some_and(|draft| !draft.is_empty()),
    }
}

#[cfg(test)]
mod tests;
