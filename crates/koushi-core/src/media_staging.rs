//! Core-owned staged-upload orchestration.
//!
//! This module owns the lifecycle around the reducer and the in-memory
//! preparation registry. Byte preparation is deliberately detached from the
//! registry lock; only the short state/registry commit sections are serialized.

use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(any(test, feature = "test-hooks"))]
use tokio::sync::oneshot;

use koushi_state::{
    ComposerDocument, ComposerDraftRevision, ComposerTarget, ImageUploadCompressionPolicy,
    StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind, StagedUploadOutputSelection,
    StagedUploadPreparation,
};

use koushi_protocol::{
    AccountKey, AppCommand, CoreCommand, TimelineCommand, UploadMediaKind, UploadMediaRequest,
};

use crate::{
    CoreConnection, OutcomeCorrelation, RequestOutcome, RequestOutcomeError,
    RequestOutcomeExpectation, executor,
    media_preparation::{
        MAX_PREPARATION_BATCH_SIZE, MediaPreparationRegistry, MediaPreparationService,
        StageUploadBytesInput,
    },
};

/// Maximum total source bytes accepted by one staging request.
pub const MAX_MEDIA_STAGING_BATCH_BYTES: usize = 128 * 1024 * 1024;
/// Maximum number of source items accepted by one staging request.
pub const MAX_MEDIA_STAGING_BATCH_SIZE: usize = MAX_PREPARATION_BATCH_SIZE;
const MEDIA_STAGING_TIMEOUT: Duration = Duration::from_secs(5);
const PREPARED_MEDIA_QUEUE_TIMEOUT: Duration = Duration::from_secs(10);
const COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_PREPARED_MEDIA_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);
static PREPARED_MEDIA_PROCESS_NONCE: OnceLock<u128> = OnceLock::new();

fn next_prepared_media_transaction_id() -> String {
    let nonce = *PREPARED_MEDIA_PROCESS_NONCE.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos()
            ^ u128::from(std::process::id())
    });
    format!(
        "desktop-prepared-media-{nonce:x}-{}",
        NEXT_PREPARED_MEDIA_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MediaStagingError {
    #[error("media staging batch is empty")]
    BatchEmpty,
    #[error("media staging batch exceeds its limit")]
    BatchTooLarge,
    #[error("media staging contains a duplicate staged id")]
    DuplicateStagedId,
    #[error("media staging positions must be nonzero and unique")]
    InvalidPosition,
    #[error("media staging target is no longer active")]
    TargetInactive,
    #[error("staged upload item is not ready for this operation")]
    PreparationNotReady,
    #[error("staged upload selection is invalid for this item")]
    InvalidSelection,
    #[error("staged upload compression choice is invalid for this item")]
    InvalidCompressionChoice,
    #[error("staged upload item is no longer available")]
    MissingStagedItem,
    #[error("prepared media bytes are no longer available")]
    PreparedBytesUnavailable,
    #[error("media preparation did not produce an output")]
    PreparationFailed,
    #[error("media preparation task did not complete")]
    PreparationTask,
    #[error("media staging became stale")]
    Stale,
    #[error("media staging command could not be submitted: {0}")]
    CommandSubmit(crate::CommandSubmitError),
    #[error("media staging outcome was not observed: {0}")]
    Outcome(RequestOutcomeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PreparedUploadSendError {
    #[error("prepared upload account is not active")]
    AccountMismatch,
    #[error("prepared upload target is not active")]
    TargetInactive,
    #[error("prepared uploads are not sendable")]
    NotSendable,
    #[error("prepared upload bytes are no longer available")]
    PreparedBytesUnavailable,
    #[error("composer draft revision is stale or exhausted")]
    DraftRevision,
    #[error("composer draft permit is invalid")]
    ComposerPermit,
    #[error("prepared upload item is no longer available")]
    StaleItem,
    #[error("prepared upload command could not be submitted: {0}")]
    CommandSubmit(crate::CommandSubmitError),
    #[error("prepared upload outcome was not observed: {0}")]
    Outcome(RequestOutcomeError),
}

pub struct PreparedUploadSendResult {
    pub accepted_revision: ComposerDraftRevision,
    /// Published generation at acceptance; state is inspected separately.
    ///
    /// ```
    /// # fn inspect(result: &koushi_core::media_staging::PreparedUploadSendResult) {
    /// let generation: u64 = result.generation;
    /// # }
    /// ```
    pub generation: u64,
}

#[derive(Clone)]
pub struct MediaStagingService {
    preparation: Arc<MediaPreparationService>,
    target_admissions: Arc<StdMutex<BTreeMap<ComposerTarget, Arc<tokio::sync::Mutex<()>>>>>,
    #[cfg(any(test, feature = "test-hooks"))]
    preparation_pause: Arc<std::sync::Mutex<Option<PreparationPause>>>,
}

#[cfg(any(test, feature = "test-hooks"))]
struct PreparationPause {
    started: oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(any(test, feature = "test-hooks"))]
pub struct PreparationBarrierForTesting {
    started: oneshot::Receiver<()>,
    release: Option<std::sync::mpsc::Sender<()>>,
}

#[cfg(any(test, feature = "test-hooks"))]
impl PreparationBarrierForTesting {
    pub async fn wait_started(&mut self) {
        (&mut self.started)
            .await
            .expect("preparation barrier must start");
    }

    pub fn release(mut self) {
        self.release
            .take()
            .expect("preparation barrier releases once")
            .send(())
            .expect("preparation must be waiting at barrier");
    }
}

impl MediaStagingService {
    pub(crate) fn new(preparation: Arc<MediaPreparationService>) -> Self {
        Self {
            preparation,
            target_admissions: Arc::new(StdMutex::new(BTreeMap::new())),
            #[cfg(any(test, feature = "test-hooks"))]
            preparation_pause: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    async fn admit_target(&self, target: &ComposerTarget) -> tokio::sync::OwnedMutexGuard<()> {
        let admission = {
            let mut admissions = self
                .target_admissions
                .lock()
                .expect("media staging target admission mutex");
            Arc::clone(
                admissions
                    .entry(target.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        admission.lock_owned().await
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn install_preparation_barrier_for_testing(&self) -> PreparationBarrierForTesting {
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *self
            .preparation_pause
            .lock()
            .expect("preparation barrier mutex") = Some(PreparationPause {
            started,
            release: release_rx,
        });
        PreparationBarrierForTesting {
            started: started_rx,
            release: Some(release),
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn pause_preparation_for_testing(&self) {
        let pause = self
            .preparation_pause
            .lock()
            .expect("preparation barrier mutex")
            .take();
        if let Some(pause) = pause {
            let _ = pause.started.send(());
            let _ = pause.release.recv();
        }
    }

    /// Publish Preparing, prepare bytes off-lock, then publish the exact
    /// prepared result through the existing AppCommand/reducer path.
    /// Return the published generation after this operation settles.
    pub async fn stage_upload_bytes(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        items: Vec<StageUploadBytesInput>,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        validate_batch(&items)?;
        let initial = connection.snapshot();
        let policy = authoritative_policy(&initial);
        let initial_account = account_key(&initial);
        let existing = active_items(&initial, &target).ok_or(MediaStagingError::TargetInactive)?;
        let existing_ids = existing
            .iter()
            .map(|item| item.staged_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut seen = std::collections::BTreeSet::new();
        let mut positions = existing
            .iter()
            .map(|item| item.position)
            .collect::<std::collections::BTreeSet<_>>();
        for item in &items {
            if !seen.insert(item.staged_id.as_str())
                || existing_ids.contains(item.staged_id.as_str())
            {
                return Err(MediaStagingError::DuplicateStagedId);
            }
            if item.position == 0 || !positions.insert(item.position) {
                return Err(MediaStagingError::InvalidPosition);
            }
        }

        let mut preparing_items = existing
            .iter()
            .cloned()
            .chain(items.iter().map(|item| preparing_item(&target, item)))
            .collect::<Vec<_>>();
        preparing_items.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.staged_id.cmp(&right.staged_id))
        });
        let preparing_ids = ids(&preparing_items);
        self.publish(
            connection,
            initial_account.clone(),
            target.clone(),
            preparing_items,
            preparing_ids.clone(),
            false,
        )
        .await?;

        let preparation_target = target.clone();
        let prepared_inputs = items;
        #[cfg(any(test, feature = "test-hooks"))]
        let preparation_service = self.clone();
        let prepared = executor::spawn_blocking(move || {
            #[cfg(any(test, feature = "test-hooks"))]
            preparation_service.pause_preparation_for_testing();
            let mut registry = MediaPreparationRegistry::default();
            let items = registry.prepare_items(&preparation_target, prepared_inputs, policy);
            (registry, items)
        })
        .await
        .map_err(|_| MediaStagingError::PreparationTask)?;
        let (prepared_registry, prepared_items) = prepared;

        let current = connection.snapshot();
        let current_items = active_items(&current, &target).ok_or(MediaStagingError::Stale)?;
        if account_key(&current) != initial_account
            || authoritative_policy(&current) != policy
            || ids(current_items) != preparing_ids
            || prepared_items.iter().any(|prepared| {
                !current_items.iter().any(|current| {
                    current.staged_id == prepared.staged_id
                        && matches!(current.preparation, StagedUploadPreparation::Preparing)
                })
            })
        {
            return Err(MediaStagingError::Stale);
        }

        let mut prepared_by_id = prepared_items
            .into_iter()
            .map(|item| (item.staged_id.clone(), item))
            .collect::<BTreeMap<_, _>>();
        let mut ready_items = current_items.to_vec();
        for item in &mut ready_items {
            if let Some(mut prepared) = prepared_by_id.remove(&item.staged_id) {
                // Captions and user-selected compression are state-owned
                // metadata, not preparation output.
                prepared.caption = item.caption.clone();
                prepared.compression_choice = item.compression_choice;
                *item = prepared;
            }
        }
        if !prepared_by_id.is_empty() {
            return Err(MediaStagingError::Stale);
        }
        self.merge_after_revalidation(prepared_registry).await;
        self.publish(
            connection,
            initial_account,
            target,
            ready_items.clone(),
            ids(&ready_items),
            false,
        )
        .await
    }

    /// Return the published generation after this operation settles.
    pub async fn select_staged_upload_output(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        staged_id: String,
        selection: StagedUploadOutputSelection,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let initial = connection.snapshot();
        let policy = authoritative_policy(&initial);
        let account = account_key(&initial);
        if !target_is_active(&initial, &target) {
            return Err(MediaStagingError::TargetInactive);
        }
        let item = staged_item(&initial, &target, &staged_id)
            .ok_or(MediaStagingError::MissingStagedItem)?;
        if !matches!(item.preparation, StagedUploadPreparation::Ready { .. }) {
            return Err(MediaStagingError::PreparationNotReady);
        }
        if !matches!(item.kind, StagedUploadKind::Image { .. }) {
            return Err(MediaStagingError::InvalidSelection);
        }
        let selected = self
            .publish_selection(
                connection,
                account.clone(),
                target.clone(),
                staged_id.clone(),
                selection,
            )
            .await?;

        let cached = {
            let mut transition = self.preparation.transition().await;
            transition.select_variant(
                &target,
                &staged_id,
                &MediaPreparationRegistry::output_identity(selection),
            )
        };
        if cached {
            return Ok(selected);
        }

        let current = connection.snapshot();
        let generation = staged_item(&current, &target, &staged_id)
            .and_then(|item| match item.preparation {
                StagedUploadPreparation::Ready { generation, .. } => Some(generation),
                StagedUploadPreparation::Preparing | StagedUploadPreparation::Failed { .. } => None,
            })
            .ok_or(MediaStagingError::MissingStagedItem)?;
        let source = {
            let transition = self.preparation.transition().await;
            transition
                .source_input(&target, &staged_id)
                .ok_or(MediaStagingError::PreparedBytesUnavailable)?
        };
        #[cfg(any(test, feature = "test-hooks"))]
        let preparation_service = self.clone();
        let encoded = executor::spawn_blocking(move || {
            #[cfg(any(test, feature = "test-hooks"))]
            preparation_service.pause_preparation_for_testing();
            MediaPreparationRegistry::encode_output(&source, selection, policy)
        })
        .await
        .map_err(|_| MediaStagingError::PreparationTask)?
        .ok_or(MediaStagingError::PreparationFailed)?;
        let (descriptor, bytes) = encoded;

        let current = connection.snapshot();
        let item = staged_item(&current, &target, &staged_id).ok_or(MediaStagingError::Stale)?;
        if account_key(&current) != account
            || authoritative_policy(&current) != policy
            || !target_is_active(&current, &target)
            || !matches!(
                item.preparation,
                StagedUploadPreparation::Ready { generation: current_generation, .. }
                    if current_generation == generation
            )
        {
            return Err(MediaStagingError::Stale);
        }
        let replacement = koushi_state::staged_upload_item_with_completed_output(
            item,
            descriptor.clone(),
            generation,
        )
        .ok_or(MediaStagingError::Stale)?;
        {
            let mut transition = self.preparation.transition().await;
            transition.insert_prepared_output(&target, &staged_id, descriptor, bytes);
        }
        self.replace_staged_upload_item(connection, account, target, staged_id, replacement)
            .await
    }

    /// Return the published generation after this operation settles.
    pub async fn retry_staged_upload_preparation(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        staged_id: String,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let initial = connection.snapshot();
        let policy = authoritative_policy(&initial);
        let account = account_key(&initial);
        if !target_is_active(&initial, &target) {
            return Err(MediaStagingError::TargetInactive);
        }
        let item = staged_item(&initial, &target, &staged_id)
            .ok_or(MediaStagingError::MissingStagedItem)?;
        if !matches!(item.preparation, StagedUploadPreparation::Failed { .. }) {
            return Err(MediaStagingError::PreparationNotReady);
        }
        let source = {
            let transition = self.preparation.transition().await;
            transition
                .source_input(&target, &staged_id)
                .ok_or(MediaStagingError::PreparedBytesUnavailable)?
        };
        let retry_target = target.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let preparation_service = self.clone();
        let (prepared_registry, replacement) = executor::spawn_blocking(move || {
            #[cfg(any(test, feature = "test-hooks"))]
            preparation_service.pause_preparation_for_testing();
            let mut registry = MediaPreparationRegistry::default();
            let replacement = registry
                .prepare_items(&retry_target, vec![source], policy)
                .into_iter()
                .next();
            (registry, replacement)
        })
        .await
        .map_err(|_| MediaStagingError::PreparationTask)?;
        let replacement = replacement.ok_or(MediaStagingError::PreparationFailed)?;
        let current = connection.snapshot();
        if account_key(&current) != account
            || authoritative_policy(&current) != policy
            || !target_is_active(&current, &target)
        {
            return Err(MediaStagingError::Stale);
        }
        let current_item = staged_item(&current, &target, &staged_id)
            .filter(|item| matches!(item.preparation, StagedUploadPreparation::Failed { .. }))
            .ok_or(MediaStagingError::Stale)?;
        let mut replacement = replacement;
        replacement.caption = current_item.caption.clone();
        replacement.compression_choice = current_item.compression_choice;
        if replacement == *current_item {
            return Ok(connection.state_generation());
        }
        {
            let mut transition = self.preparation.transition().await;
            transition.remove_item(&target, &staged_id);
            transition.merge_prepared(prepared_registry);
        }
        self.replace_staged_upload_item(connection, account, target, staged_id, replacement)
            .await
    }

    /// Return the published generation after this operation settles.
    pub async fn use_original(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        staged_id: String,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let snapshot = connection.snapshot();
        let account = account_key(&snapshot);
        if !target_is_active(&snapshot, &target) {
            return Err(MediaStagingError::TargetInactive);
        }
        if staged_item(&snapshot, &target, &staged_id).is_none() {
            return Err(MediaStagingError::MissingStagedItem);
        }
        let replacement = {
            let mut transition = self.preparation.transition().await;
            transition
                .use_original(&target, &staged_id)
                .ok_or(MediaStagingError::PreparedBytesUnavailable)?
        };
        self.replace_staged_upload_item(connection, account, target, staged_id, replacement)
            .await
    }

    /// Return the published generation after this operation settles.
    pub async fn update_caption(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        staged_id: String,
        caption: Option<ComposerDocument>,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let snapshot = connection.snapshot();
        let account = account_key(&snapshot);
        let staged_id = staged_id.clone();
        if !target_is_active(&snapshot, &target) {
            return Err(MediaStagingError::TargetInactive);
        }
        if staged_item(&snapshot, &target, &staged_id).is_none() {
            return Err(MediaStagingError::MissingStagedItem);
        }
        let expected_ids = active_ids(&snapshot, &target)?;
        let caption = caption
            .and_then(|document| (!document.plain_body().trim().is_empty()).then_some(document));
        let request_id = connection.next_request_id();
        let baseline = connection.state_generation();
        connection
            .command(CoreCommand::App(AppCommand::UpdateStagedUploadCaption {
                request_id,
                target: target.clone(),
                staged_id,
                caption,
            }))
            .await
            .map_err(MediaStagingError::CommandSubmit)?;
        self.wait(
            connection,
            request_id,
            account,
            target,
            expected_ids,
            baseline,
            false,
        )
        .await
    }

    /// Return the published generation after this operation settles.
    pub async fn update_compression(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        staged_id: String,
        compression_choice: StagedUploadCompressionChoice,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let snapshot = connection.snapshot();
        let account = account_key(&snapshot);
        if !target_is_active(&snapshot, &target) {
            return Err(MediaStagingError::TargetInactive);
        }
        if staged_item(&snapshot, &target, &staged_id).is_none() {
            return Err(MediaStagingError::MissingStagedItem);
        }
        let expected_ids = active_ids(&snapshot, &target)?;
        let request_id = connection.next_request_id();
        let baseline = connection.state_generation();
        connection
            .command(CoreCommand::App(
                AppCommand::UpdateStagedUploadCompression {
                    request_id,
                    target: target.clone(),
                    staged_id,
                    compression_choice,
                },
            ))
            .await
            .map_err(MediaStagingError::CommandSubmit)?;
        self.wait(
            connection,
            request_id,
            account,
            target,
            expected_ids,
            baseline,
            false,
        )
        .await
    }

    /// Return the published generation after this operation settles.
    pub async fn clear(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
    ) -> Result<u64, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let snapshot = connection.snapshot();
        let account = account_key(&snapshot);
        let Some(items) = active_items(&snapshot, &target) else {
            return Err(MediaStagingError::TargetInactive);
        };
        if items.is_empty() {
            self.preparation.reconcile_snapshot(&snapshot).await;
            return Ok(connection.state_generation());
        }
        let request_id = connection.next_request_id();
        let baseline = connection.state_generation();
        connection
            .command(CoreCommand::App(AppCommand::ClearUploadStaging {
                request_id,
                target: target.clone(),
            }))
            .await
            .map_err(MediaStagingError::CommandSubmit)?;
        let result = self
            .wait(
                connection,
                request_id,
                account,
                target,
                Vec::new(),
                baseline,
                false,
            )
            .await?;
        // Reconcile current state, which may be newer than the settled generation.
        let snapshot = connection.snapshot();
        self.preparation.reconcile_snapshot(&snapshot).await;
        Ok(result)
    }

    pub async fn prepared_upload_preview(
        &self,
        connection: &mut CoreConnection,
        target: ComposerTarget,
        staged_id: String,
        variant_id: String,
    ) -> Result<Vec<u8>, MediaStagingError> {
        let _admission = self.admit_target(&target).await;
        let snapshot = connection.snapshot();
        let account = ready_account(&snapshot);
        let item = staged_item(&snapshot, &target, &staged_id)
            .ok_or(MediaStagingError::MissingStagedItem)?;
        let has_variant = matches!(
            &item.preparation,
            StagedUploadPreparation::Ready { variants, .. }
                if variants.iter().any(|variant| variant.variant_id == variant_id)
        );
        if !has_variant {
            return Err(MediaStagingError::PreparedBytesUnavailable);
        }
        let bytes = self
            .preparation
            .transition()
            .await
            .variant_bytes(&target, &staged_id, &variant_id)
            .ok_or(MediaStagingError::PreparedBytesUnavailable)?;
        let current = connection.snapshot();
        let current_item = staged_item(&current, &target, &staged_id);
        let current_has_variant = current_item.is_some_and(|item| {
            matches!(
                &item.preparation,
                StagedUploadPreparation::Ready { variants, .. }
                    if variants.iter().any(|variant| variant.variant_id == variant_id)
            )
        });
        if ready_account(&current) != account
            || !target_is_active(&current, &target)
            || !current_has_variant
        {
            return Err(MediaStagingError::Stale);
        }
        Ok(bytes)
    }

    pub async fn send_prepared_uploads(
        &self,
        connection: &mut CoreConnection,
        expected_account: koushi_protocol::SessionKeyId,
        generation: crate::composer_draft_lifecycle::ComposerRendererGeneration,
        lease: crate::composer_draft_lifecycle::ComposerDraftLeaseId,
        target: ComposerTarget,
        draft_revision: ComposerDraftRevision,
    ) -> Result<PreparedUploadSendResult, PreparedUploadSendError> {
        let _admission = self.admit_target(&target).await;
        let initial = connection.snapshot();
        if ready_account(&initial).as_ref() != Some(&expected_account) {
            return Err(PreparedUploadSendError::AccountMismatch);
        }
        if !target_is_active(&initial, &target) {
            return Err(PreparedUploadSendError::TargetInactive);
        }
        let staged_ids = active_items(&initial, &target)
            .filter(|items| !items.is_empty() && koushi_state::staged_uploads_are_sendable(items))
            .map(|items| ids(items))
            .ok_or(PreparedUploadSendError::NotSendable)?;
        let expected_revision = next_acceptance_revision(&initial, &target, draft_revision)
            .ok_or(PreparedUploadSendError::DraftRevision)?;
        let _permit = connection
            .acquire_composer_draft_command_permit(
                generation,
                lease,
                &crate::composer_draft_lifecycle::ComposerDraftScope {
                    account: expected_account.clone(),
                    target: target.clone(),
                },
            )
            .map_err(|_| PreparedUploadSendError::ComposerPermit)?;
        let account_key = AccountKey(expected_account.user_id.clone());
        let key = timeline_key(account_key.clone(), &target);

        for staged_id in staged_ids {
            let current = connection.snapshot();
            if ready_account(&current).as_ref() != Some(&expected_account) {
                return Err(PreparedUploadSendError::AccountMismatch);
            }
            if composer_draft_revision(&current, &target) != draft_revision {
                return Err(PreparedUploadSendError::DraftRevision);
            }
            let item = staged_item(&current, &target, &staged_id)
                .filter(|item| {
                    koushi_state::staged_uploads_are_sendable(std::slice::from_ref(item))
                })
                .ok_or(PreparedUploadSendError::StaleItem)?;
            let prepared = self
                .preparation
                .transition()
                .await
                .selected_upload(&target, &staged_id)
                .ok_or(PreparedUploadSendError::PreparedBytesUnavailable)?;
            let request_id = connection.next_request_id();
            let transaction_id = next_prepared_media_transaction_id();
            let descriptor = prepared.descriptor;
            let kind = if descriptor.mime_type.starts_with("image/") {
                UploadMediaKind::Image {
                    width: descriptor.width,
                    height: descriptor.height,
                }
            } else {
                UploadMediaKind::File
            };
            let baseline = connection.state_generation();
            connection
                .command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
                    request_id,
                    expected_account: expected_account.clone(),
                    key: key.clone(),
                    transaction_id: transaction_id.clone(),
                    request: UploadMediaRequest {
                        filename: descriptor.filename,
                        mime_type: descriptor.mime_type,
                        bytes: prepared.bytes,
                        kind,
                        compression: None,
                        thumbnail: None,
                        caption: media_caption_from_composer_document(
                            item.caption.as_ref(),
                            current.settings.values.composer.formatting_options(),
                        ),
                    },
                }))
                .await
                .map_err(PreparedUploadSendError::CommandSubmit)?;
            self.wait_prepared_media_queue(
                connection,
                request_id,
                key.clone(),
                transaction_id,
                baseline,
            )
            .await?;

            let current = connection.snapshot();
            if ready_account(&current).as_ref() != Some(&expected_account)
                || !target_is_active(&current, &target)
                || staged_item(&current, &target, &staged_id).is_none()
            {
                return Err(PreparedUploadSendError::StaleItem);
            }
            let mut remaining = active_items(&current, &target)
                .ok_or(PreparedUploadSendError::TargetInactive)?
                .to_vec();
            remaining.retain(|item| item.staged_id != staged_id);
            let remaining_ids = ids(&remaining);
            self.publish(
                connection,
                account_key.clone(),
                target.clone(),
                remaining,
                remaining_ids,
                false,
            )
            .await
            .map_err(map_staging_send_error)?;
            self.preparation
                .transition()
                .await
                .remove_item(&target, &staged_id);
        }

        let current = connection.snapshot();
        if ready_account(&current).as_ref() != Some(&expected_account) {
            return Err(PreparedUploadSendError::AccountMismatch);
        }
        if !target_is_active(&current, &target)
            || composer_draft_revision(&current, &target) != draft_revision
        {
            return Err(PreparedUploadSendError::DraftRevision);
        }
        let request_id = connection.next_request_id();
        let baseline = connection.state_generation();
        connection
            .command_with_composer_lease(
                generation,
                lease,
                CoreCommand::App(AppCommand::AcceptComposerDraft {
                    request_id,
                    expected_account,
                    target: target.clone(),
                    submitted_revision: draft_revision,
                }),
            )
            .await
            .map_err(PreparedUploadSendError::CommandSubmit)?;
        let outcome = connection
            .wait_for_request_outcome(
                OutcomeCorrelation::Request(request_id),
                RequestOutcomeExpectation::ComposerAccepted {
                    request_id,
                    account_key,
                    target,
                    expected_revision,
                },
                baseline,
                executor::Instant::now() + COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT,
            )
            .await
            .map_err(PreparedUploadSendError::Outcome)?;
        match outcome {
            RequestOutcome::ComposerAccepted {
                revision,
                generation,
                ..
            } => Ok(PreparedUploadSendResult {
                accepted_revision: revision,
                generation,
            }),
            _ => Err(PreparedUploadSendError::Outcome(
                RequestOutcomeError::InvalidOutcome,
            )),
        }
    }

    async fn wait_prepared_media_queue(
        &self,
        connection: &mut CoreConnection,
        request_id: koushi_protocol::ids::RequestId,
        key: koushi_protocol::ids::TimelineKey,
        transaction_id: String,
        baseline_generation: u64,
    ) -> Result<(), PreparedUploadSendError> {
        match connection
            .wait_for_request_outcome(
                OutcomeCorrelation::Request(request_id),
                RequestOutcomeExpectation::PreparedMediaQueued {
                    request_id,
                    key,
                    transaction_id,
                },
                baseline_generation,
                executor::Instant::now() + PREPARED_MEDIA_QUEUE_TIMEOUT,
            )
            .await
            .map_err(PreparedUploadSendError::Outcome)?
        {
            RequestOutcome::PreparedMediaQueued { .. } => Ok(()),
            _ => Err(PreparedUploadSendError::Outcome(
                RequestOutcomeError::InvalidOutcome,
            )),
        }
    }

    async fn publish(
        &self,
        connection: &mut CoreConnection,
        account: AccountKey,
        target: ComposerTarget,
        items: Vec<StagedUploadItem>,
        expected_ids: Vec<String>,
        allow_initial: bool,
    ) -> Result<u64, MediaStagingError> {
        let request_id = connection.next_request_id();
        let baseline = connection.state_generation();
        connection
            .command(CoreCommand::App(AppCommand::SetUploadStaging {
                request_id,
                target: target.clone(),
                items,
            }))
            .await
            .map_err(MediaStagingError::CommandSubmit)?;
        self.wait(
            connection,
            request_id,
            account,
            target,
            expected_ids,
            baseline,
            allow_initial,
        )
        .await
    }

    async fn publish_selection(
        &self,
        connection: &mut CoreConnection,
        account: AccountKey,
        target: ComposerTarget,
        staged_id: String,
        selection: StagedUploadOutputSelection,
    ) -> Result<u64, MediaStagingError> {
        let expected_ids = active_ids(&connection.snapshot(), &target)?;
        let request_id = connection.next_request_id();
        let baseline = connection.state_generation();
        connection
            .command(CoreCommand::App(AppCommand::SelectStagedUploadOutput {
                request_id,
                target: target.clone(),
                staged_id,
                selection,
            }))
            .await
            .map_err(MediaStagingError::CommandSubmit)?;
        self.wait(
            connection,
            request_id,
            account,
            target,
            expected_ids,
            baseline,
            false,
        )
        .await
    }

    async fn replace_staged_upload_item(
        &self,
        connection: &mut CoreConnection,
        account: AccountKey,
        target: ComposerTarget,
        staged_id: String,
        replacement: StagedUploadItem,
    ) -> Result<u64, MediaStagingError> {
        let current = connection.snapshot();
        let mut items = active_items(&current, &target)
            .ok_or(MediaStagingError::TargetInactive)?
            .to_vec();
        let item = items
            .iter_mut()
            .find(|item| item.staged_id == staged_id)
            .ok_or(MediaStagingError::MissingStagedItem)?;
        *item = replacement;
        let expected_ids = ids(&items);
        self.publish(connection, account, target, items, expected_ids, false)
            .await
    }

    async fn merge_after_revalidation(&self, prepared: MediaPreparationRegistry) {
        self.preparation.transition().await.merge_prepared(prepared);
    }

    async fn wait(
        &self,
        connection: &mut CoreConnection,
        request_id: koushi_protocol::ids::RequestId,
        account_key: AccountKey,
        target: ComposerTarget,
        staged_ids: Vec<String>,
        baseline_generation: u64,
        allow_initial: bool,
    ) -> Result<u64, MediaStagingError> {
        match connection
            .wait_for_request_outcome(
                OutcomeCorrelation::Request(request_id),
                RequestOutcomeExpectation::UploadStaging {
                    request_id,
                    account_key,
                    target,
                    staged_ids,
                    allow_initial,
                },
                baseline_generation,
                executor::Instant::now() + MEDIA_STAGING_TIMEOUT,
            )
            .await
            .map_err(MediaStagingError::Outcome)?
        {
            RequestOutcome::UploadStaging { generation, .. } => Ok(generation),
            _ => Err(MediaStagingError::Outcome(
                RequestOutcomeError::InvalidOutcome,
            )),
        }
    }
}

fn map_staging_send_error(error: MediaStagingError) -> PreparedUploadSendError {
    match error {
        MediaStagingError::CommandSubmit(error) => PreparedUploadSendError::CommandSubmit(error),
        MediaStagingError::Outcome(error) => PreparedUploadSendError::Outcome(error),
        MediaStagingError::TargetInactive => PreparedUploadSendError::TargetInactive,
        MediaStagingError::MissingStagedItem
        | MediaStagingError::Stale
        | MediaStagingError::PreparationNotReady => PreparedUploadSendError::StaleItem,
        _ => PreparedUploadSendError::StaleItem,
    }
}

fn ready_account(state: &koushi_state::AppState) -> Option<koushi_protocol::SessionKeyId> {
    match &state.session {
        koushi_state::SessionState::Ready(info) => {
            Some(crate::store::session_key_id_from_info(info))
        }
        _ => None,
    }
}

fn composer_draft_revision(
    state: &koushi_state::AppState,
    target: &ComposerTarget,
) -> ComposerDraftRevision {
    match target {
        ComposerTarget::Main { room_id } => state.composer_drafts.room_revision(room_id),
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_revision(room_id, root_event_id),
    }
}

fn next_acceptance_revision(
    state: &koushi_state::AppState,
    target: &ComposerTarget,
    submitted_revision: ComposerDraftRevision,
) -> Option<ComposerDraftRevision> {
    ComposerDraftRevision::checked_successor(
        composer_draft_revision(state, target),
        submitted_revision,
    )
    .ok()
}

fn timeline_key(
    account_key: AccountKey,
    target: &ComposerTarget,
) -> koushi_protocol::ids::TimelineKey {
    match target {
        ComposerTarget::Main { room_id } => koushi_protocol::ids::TimelineKey {
            account_key,
            kind: koushi_protocol::ids::TimelineKind::Room {
                room_id: room_id.clone(),
            },
        },
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => koushi_protocol::ids::TimelineKey {
            account_key,
            kind: koushi_protocol::ids::TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            },
        },
    }
}

fn media_caption_from_composer_document(
    document: Option<&ComposerDocument>,
    formatting_options: koushi_state::ComposerFormattingOptions,
) -> Option<koushi_state::FormattedMessageDraft> {
    let document = document?;
    let plain_body = document.plain_body();
    (!plain_body.trim().is_empty()).then(|| koushi_state::FormattedMessageDraft {
        plain_body,
        formatted_body: document.formatted_body_with_options(formatting_options),
        mentions: document.mention_intent(),
    })
}

fn validate_batch(items: &[StageUploadBytesInput]) -> Result<(), MediaStagingError> {
    if items.is_empty() {
        return Err(MediaStagingError::BatchEmpty);
    }
    if items.len() > MAX_MEDIA_STAGING_BATCH_SIZE {
        return Err(MediaStagingError::BatchTooLarge);
    }
    let total = items
        .iter()
        .try_fold(0usize, |total, item| total.checked_add(item.bytes.len()))
        .ok_or(MediaStagingError::BatchTooLarge)?;
    if total > MAX_MEDIA_STAGING_BATCH_BYTES {
        return Err(MediaStagingError::BatchTooLarge);
    }
    Ok(())
}

fn authoritative_policy(state: &koushi_state::AppState) -> ImageUploadCompressionPolicy {
    state.settings.values.media.image_upload_compression_policy
}

fn preparing_item(target: &ComposerTarget, input: &StageUploadBytesInput) -> StagedUploadItem {
    let mime_type = crate::media_preparation::normalized_mime(&input.mime_type);
    let kind = if mime_type.to_ascii_lowercase().starts_with("image/") {
        StagedUploadKind::Image {
            width: None,
            height: None,
        }
    } else {
        StagedUploadKind::File
    };
    StagedUploadItem {
        staged_id: input.staged_id.clone(),
        room_id: target.room_id().to_owned(),
        position: input.position,
        filename: input.filename.clone(),
        mime_type,
        byte_count: input.bytes.len() as u64,
        kind,
        caption: None,
        compression_choice: StagedUploadCompressionChoice::NotApplicable,
        preparation: StagedUploadPreparation::Preparing,
    }
}

fn target_is_active(state: &koushi_state::AppState, target: &ComposerTarget) -> bool {
    matches!(state.session, koushi_state::SessionState::Ready(_))
        && active_items(state, target).is_some()
}

fn active_items<'a>(
    state: &'a koushi_state::AppState,
    target: &ComposerTarget,
) -> Option<&'a [StagedUploadItem]> {
    if !matches!(state.session, koushi_state::SessionState::Ready(_)) {
        return None;
    }
    match target {
        ComposerTarget::Main { room_id }
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) =>
        {
            Some(&state.timeline.staged_uploads)
        }
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => match &state.thread {
            koushi_state::ThreadPaneState::Open {
                room_id: current_room,
                root_event_id: current_root,
                staged_uploads,
                ..
            } if current_room == room_id && current_root == root_event_id => Some(staged_uploads),
            _ => None,
        },
        _ => None,
    }
}

fn active_ids(
    state: &koushi_state::AppState,
    target: &ComposerTarget,
) -> Result<Vec<String>, MediaStagingError> {
    active_items(state, target)
        .map(ids)
        .ok_or(MediaStagingError::TargetInactive)
}

fn staged_item<'a>(
    state: &'a koushi_state::AppState,
    target: &ComposerTarget,
    staged_id: &str,
) -> Option<&'a StagedUploadItem> {
    active_items(state, target)?
        .iter()
        .find(|item| item.staged_id == staged_id)
}

fn ids(items: &[StagedUploadItem]) -> Vec<String> {
    items.iter().map(|item| item.staged_id.clone()).collect()
}

fn account_key(state: &koushi_state::AppState) -> AccountKey {
    let user_id = match &state.session {
        koushi_state::SessionState::Ready(info)
        | koushi_state::SessionState::Locked(info)
        | koushi_state::SessionState::CapabilityBlocked { info, .. }
        | koushi_state::SessionState::SwitchingAccount { info }
        | koushi_state::SessionState::Provisional { info, .. }
        | koushi_state::SessionState::AwaitingVerification { info, .. }
        | koushi_state::SessionState::Verifying { info, .. }
        | koushi_state::SessionState::AwaitingBootstrapConfirmation { info, .. }
        | koushi_state::SessionState::Rejecting { info, .. } => info.user_id.clone(),
        _ => String::new(),
    };
    AccountKey(user_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::{ComposerFormattingOptions, ComposerInline, MentionTarget};

    #[test]
    fn caption_document_converts_at_the_core_media_send_boundary() {
        let document = ComposerDocument::new(vec![
            ComposerInline::Text {
                text: "**hello** ".to_owned(),
            },
            ComposerInline::Mention {
                target: MentionTarget::User {
                    user_id: "@alice:example.invalid".to_owned(),
                    display_label: "Alice".to_owned(),
                },
                display_label: "Alice".to_owned(),
            },
        ]);
        let draft = media_caption_from_composer_document(
            Some(&document),
            ComposerFormattingOptions { math_mode: true },
        )
        .expect("non-empty caption");

        assert_eq!(draft.plain_body, "**hello** @Alice");
        let formatted = draft.formatted_body.as_deref().unwrap_or_default();
        assert!(formatted.contains("<strong>"));
        assert!(formatted.contains("https://matrix.to/#/%40alice%3Aexample.invalid"));
        assert_eq!(draft.mentions, document.mention_intent());
        assert!(
            media_caption_from_composer_document(
                Some(&ComposerDocument::from_plain_text("  \n  ")),
                ComposerFormattingOptions::default(),
            )
            .is_none()
        );
    }
}
