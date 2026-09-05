//! `CoreRuntime`, `CoreConnection`, and the `AppActor` loop.
//!
//! Channel topology (overview.md, Async rule 10):
//! - command inbox per runtime: bounded mpsc, capacity 256
//! - discrete core events per consumer: broadcast, capacity 16384; a lagged
//!   consumer observes `EventStreamLag` and resyncs from the snapshot watch
//! - state snapshots: latest-wins watch, coalesced to at most one
//!   `StateDelta` per processed command batch

mod activity;
mod composer;
mod connection;
mod navigation;
mod profile_display_diagnostics;
mod reducer_support;
pub mod request_outcome;
mod scheduled_send;
#[cfg(test)]
mod secure_backup_admission_tests;

pub use composer::COMPOSER_DRAFT_PERSIST_DEBOUNCE;
use composer::{
    ComposerAcceptanceIdentity, ComposerDraftLoadStatus, PendingComposerAcceptance,
    PendingComposerDraftPersist, composer_acceptance_identity_for_action,
    composer_acceptance_identity_for_timeline_command, composer_draft_acceptance_would_exhaust,
    composer_draft_account_matches, composer_draft_session_key,
    timeline_submission_revision_exhaustion,
};
use navigation::{
    EventNavigationPrepared, NavigationPersistenceStatus, NavigationReplacementRoomForCleanup,
    PendingEventNavigation, PendingFocusedNavigation,
    cancel_replaced_room_timeline_link_previews_key, cancel_replaced_room_timeline_pagination_key,
    command_supersedes_event_navigation, effects_open_focused_timeline,
    focused_navigation_outcome_after_reduce, navigation_replacement_room_for_cleanup,
    unsubscribe_replaced_timeline_key,
};
use scheduled_send::scheduled_send_id;

#[cfg(any(test, feature = "test-hooks"))]
pub use connection::CoreConnectionTestControl;
pub use connection::{
    CommandSubmitError, CoreCommandHandle, CoreConnection, EventNavigationError, EventStreamLag,
    SelectRoomError,
};
pub use koushi_protocol::state_update::CoreCommandAdmission;
pub use request_outcome::{
    OutcomeCorrelation, RequestOutcome, RequestOutcomeError, RequestOutcomeExpectation,
    RoomOperationKind,
};
use std::collections::{BTreeSet, HashMap};
use std::future;
use std::path::PathBuf;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, atomic::AtomicU64};
#[cfg(test)]
use std::time::Duration;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    AccountManagementOperation, ActivityRowKind, ActivityState, AppAction, AppEffect, AppState,
    ComposerDraftStore, ComposerTarget, LoginAttemptId, NavigationState, OperationFailureKind,
    ProfileUpdateRequest, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem,
    SearchScope as AppSearchScope, SecureBackupSetupAdmission, SessionState,
    SpaceMembersCommandRejection, ThreadOpenIntent, ThreadPaneState, UiEvent,
    admit_space_member_cancellation, admit_space_member_invite, admit_space_member_role,
    admit_space_members_load, reduce,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account::{AccountActorHandle, AccountMessage};
use crate::activity_resolution::ActivityResolutionRequest;
use crate::command_policy::{
    CoreCommandPolicy, native_artifact_for_account_command, native_artifact_for_command,
    search_scope_to_state, space_member_forward_failure_action, timeline_composer_account_fence,
};
use crate::composer_draft_lifecycle::{ComposerDraftCommandPermit, ComposerDraftLeaseRegistry};
pub use activity::ACTIVITY_RECENT_MAX_ROWS;
use activity::{
    ActivityProjection, activity_tab_token, cap_activity_resolution_requests,
    guard_activity_resolution_completion, normalize_activity_resolution_action,
    record_activity_transition,
};
use koushi_protocol::command::{
    AccountCommand, AppCommand, CoreCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand,
};
use koushi_protocol::event::{
    ActivityEvent, CoreEvent, IntentNoOpReason, IntentOutcome, NativeAttentionEvent, TimelineEvent,
};
use koushi_protocol::state_update::VersionedAppStateSnapshot;

use crate::executor;
use crate::native_artifact::{NativeArtifactKind, NativeArtifactPort, RejectingNativeArtifactPort};
use crate::settings::SettingsStore;
use crate::state_delta::build_state_delta;
use crate::store::{StoreActor, session_key_id_from_info};
use koushi_protocol::failure::{CoreFailure, RoomFailureKind, TimelineFailureKind};
use koushi_protocol::ids::{
    AccountKey, RequestId, RuntimeConnectionId, TimelineGeneration, TimelineKey, TimelineKind,
};

pub const COMMAND_INBOX_CAPACITY: usize = 256;
/// Per-consumer broadcast capacity. On large accounts (100+ rooms) initial and
/// room-open sync bursts can emit thousands of `CoreEvent`s faster than a
/// consumer (the Tauri forwarder, or a transient command connection waiting for
/// a correlated event) drains them. `tokio::broadcast` silently drops the
/// overflowed messages for a lagged consumer, which previously dropped a room's
/// `InitialItems` (blank timeline) and `select_room`'s correlated event ("room
/// selection did not complete"). Sized to absorb a full large-account burst;
/// genuine lag still self-heals via `EventStreamLag` -> resync.
pub const EVENT_QUEUE_CAPACITY: usize = 16384;
/// AppActor action-projection inbox. Actors project a high volume of
/// `Vec<AppAction>` here during large-account (100+ room) sync. It MUST be large
/// enough that bursts never overflow.
///
/// Lane contract:
/// - user-intent commands use the reliable command lane (`send().await`) and
///   keep request-id correlation; they are never routed through a drop-on-full
///   path;
/// - foreground active-room work (timeline subscription, pagination, visible
///   avatars) may wait on bounded actor capacity but must not wait behind
///   background crawler availability;
/// - background work (search-crawler room availability, inactive enrichment,
///   non-visible media) is latest-wins / coalesced / drop-recoverable only.
///
/// The action queue remains large because the RoomActor projects through a
/// drop-on-full `try_send`: an overflow silently drops one-shot actions such as
/// room selection (`SelectRoom`) and room-settings/member loads, which is the
/// large-account "room selection did not complete" / blank-timeline bug. See
/// the async channel-capacity rule in docs/policies/engineering-rules.md.
pub const ACTION_QUEUE_CAPACITY: usize = 16384;
const INTERNAL_RUNTIME_CONNECTION_ID: RuntimeConnectionId = RuntimeConnectionId(0);
macro_rules! trace_runtime_sync {
    ($stage:expr, [$($field:expr),* $(,)?], $($arg:tt)*) => {{
        let event = DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.runtime",
            $stage,
        )$(.field($field))*;
        record(event);
    }};
}

fn intent_outcome_token(outcome: &IntentOutcome) -> &'static str {
    match outcome {
        IntentOutcome::Committed => "committed",
        IntentOutcome::BenignNoOp(_) => "benign_no_op",
        IntentOutcome::FailedNoOp(_) => "failed_no_op",
    }
}

/// Diagnostic-only, private-data-free record of slow AppActor loop iterations.
/// A loop iteration that takes hundreds of ms (e.g. a full `self.state.clone()`
/// of a 100+ room account) starves the
/// command arm, which is why `select_room` can time out under large-account
/// sync. Logs the arm, items handled, the state-clone cost, and total time.
fn app_loop_trace(arm: &'static str, count: u32, clone_ms: u128, total: std::time::Duration) {
    let total_ms = total.as_millis();
    if total_ms < 100 {
        return;
    }
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.runtime", "app_loop")
            .field(DiagnosticField::token("arm", arm))
            .field(DiagnosticField::count("count", count as u64))
            .field(DiagnosticField::milliseconds("clone", clone_ms))
            .field(DiagnosticField::milliseconds("duration", total_ms)),
    );
}

fn record_space_member_command_rejection(
    trigger: &'static str,
    rejection: SpaceMembersCommandRejection,
) {
    let reason = match rejection {
        SpaceMembersCommandRejection::NoSelectedSpace => "no_selected_space",
        SpaceMembersCommandRejection::WrongSpace => "wrong_space",
        SpaceMembersCommandRejection::StaleGeneration => "stale_generation",
        SpaceMembersCommandRejection::InviteAlreadyInFlight => "invite_already_in_flight",
        SpaceMembersCommandRejection::CancellationAlreadyInFlight => {
            "cancellation_already_in_flight"
        }
        SpaceMembersCommandRejection::LoadBlockedByInvite => "load_blocked_by_invite",
        SpaceMembersCommandRejection::AlreadyJoined => "already_joined",
        SpaceMembersCommandRejection::AlreadyInvited => "already_invited",
        SpaceMembersCommandRejection::NotInvited => "not_invited",
        SpaceMembersCommandRejection::NotChildRoomOnly => "not_child_room_only",
        SpaceMembersCommandRejection::RoleUpdateAlreadyInFlight => "role_update_already_in_flight",
        SpaceMembersCommandRejection::RoleNotEditable => "role_not_editable",
        SpaceMembersCommandRejection::RoleTargetInvalid => "role_target_invalid",
        SpaceMembersCommandRejection::RoleOptionUnavailable => "role_option_unavailable",
        SpaceMembersCommandRejection::RoleRevisionMismatch => "role_revision_mismatch",
        SpaceMembersCommandRejection::RoleCurrentPowerMismatch => "role_current_power_mismatch",
        SpaceMembersCommandRejection::RoleConfirmationRequired => "role_confirmation_required",
        SpaceMembersCommandRejection::RoleSessionRequired => "role_session_required",
    };
    let outcome = match rejection {
        SpaceMembersCommandRejection::StaleGeneration => "stale_generation",
        SpaceMembersCommandRejection::InviteAlreadyInFlight
        | SpaceMembersCommandRejection::CancellationAlreadyInFlight
        | SpaceMembersCommandRejection::AlreadyJoined
        | SpaceMembersCommandRejection::AlreadyInvited
        | SpaceMembersCommandRejection::RoleUpdateAlreadyInFlight => "duplicate",
        SpaceMembersCommandRejection::NoSelectedSpace
        | SpaceMembersCommandRejection::WrongSpace
        | SpaceMembersCommandRejection::LoadBlockedByInvite
        | SpaceMembersCommandRejection::NotInvited
        | SpaceMembersCommandRejection::NotChildRoomOnly
        | SpaceMembersCommandRejection::RoleNotEditable
        | SpaceMembersCommandRejection::RoleTargetInvalid
        | SpaceMembersCommandRejection::RoleOptionUnavailable
        | SpaceMembersCommandRejection::RoleRevisionMismatch
        | SpaceMembersCommandRejection::RoleCurrentPowerMismatch
        | SpaceMembersCommandRejection::RoleConfirmationRequired
        | SpaceMembersCommandRejection::RoleSessionRequired => "rejected",
    };
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.space_members_projection",
            "command_rejected",
        )
        .field(DiagnosticField::token("trigger", trigger))
        .field(DiagnosticField::token("reason", reason))
        .field(DiagnosticField::token("outcome", outcome))
        .field(DiagnosticField::count("rejection_count", 1)),
    );
}

#[cfg(any(test, feature = "test-hooks"))]
#[doc(hidden)]
pub enum CoreQaCommand {
    SetLocalDeviceBlacklisted {
        request_id: RequestId,
        target: koushi_state::VerificationTarget,
        room_id: String,
        acknowledged: oneshot::Sender<Result<(), ()>>,
    },
    RefreshDeviceKeysAndAssertKnown {
        request_id: RequestId,
        target: koushi_state::VerificationTarget,
        acknowledged: oneshot::Sender<Result<(), ()>>,
    },
    AssertInboundSessionsStartAtZero {
        request_id: RequestId,
        room_id: String,
        acknowledged: oneshot::Sender<Result<usize, ()>>,
    },
    SyncOnce {
        request_id: RequestId,
    },
}

enum CoreCommandEnvelope {
    Public {
        command: CoreCommand,
        composer_permit: Option<ComposerDraftCommandPermit>,
        admission: Option<oneshot::Sender<CoreCommandAdmission>>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    Qa(CoreQaCommand),
}

#[cfg(test)]
impl CoreCommandEnvelope {
    fn command(&self) -> &CoreCommand {
        match self {
            Self::Public { command, .. } => command,
            Self::Qa(_) => panic!("expected public command envelope"),
        }
    }
}

/// A task handle that is aborted if its owner is dropped without an orderly
/// shutdown. Explicit shutdown takes the handle and awaits it; error paths in
/// headless QA and embedding callers therefore cannot leave detached runtime
/// tasks keeping the process alive indefinitely.
struct AbortOnDrop<T> {
    handle: Option<executor::JoinHandle<T>>,
}

impl<T> AbortOnDrop<T> {
    fn new(handle: executor::JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    fn get(&self) -> &executor::JoinHandle<T> {
        self.handle
            .as_ref()
            .expect("abort-on-drop task handle must remain present")
    }

    fn take(&mut self) -> executor::JoinHandle<T> {
        self.handle
            .take()
            .expect("abort-on-drop task handle must be taken once")
    }

    fn abort(&self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Owns the actor tree and creates [`CoreConnection`] handles.
pub struct CoreRuntime {
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    next_connection_id: AtomicU64,
    composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    native_artifacts: Arc<dyn NativeArtifactPort>,
    // Internal action channel: actors project side-effect outcomes through
    // the reducer with this in later phases; tests inject through it today.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    action_tx: mpsc::Sender<Vec<AppAction>>,
    #[cfg(any(test, feature = "test-hooks"))]
    composer_draft_test_tx: mpsc::Sender<ComposerDraftTestMutation>,
    /// Account-runtime-owned source and prepared variant bytes. The WebView
    /// receives descriptors only; adapters may operate on this cache through
    /// the typed runtime boundary.
    media_preparation: Arc<crate::media_preparation::MediaPreparationService>,
    media_staging: Arc<crate::media_staging::MediaStagingService>,
    media_lifecycle: AbortOnDrop<()>,
    #[cfg(any(test, feature = "test-hooks"))]
    account_actor_test_handle: AccountActorHandle,
    #[cfg(any(test, feature = "test-hooks"))]
    composer_draft_store_actor_for_testing: StoreActor,
    actor: AbortOnDrop<()>,
}

#[cfg(any(test, feature = "test-hooks"))]
#[doc(hidden)]
pub struct ComposerDraftIoBarrierForTesting {
    save_started: oneshot::Receiver<()>,
    save_release: Option<std::sync::mpsc::Sender<()>>,
    save_completed: oneshot::Receiver<()>,
    load_started: oneshot::Receiver<()>,
    load_completed: oneshot::Receiver<()>,
    load_attempt_count: Arc<AtomicUsize>,
}

#[cfg(any(test, feature = "test-hooks"))]
impl ComposerDraftIoBarrierForTesting {
    pub async fn wait_for_save_started(&mut self) {
        (&mut self.save_started)
            .await
            .expect("composer draft save-start probe must remain available");
    }

    pub fn load_attempt_count(&self) -> usize {
        self.load_attempt_count.load(Ordering::Acquire)
    }

    pub fn load_started_before_release(&mut self) -> bool {
        match self.load_started.try_recv() {
            Ok(()) => true,
            Err(oneshot::error::TryRecvError::Empty) => false,
            Err(oneshot::error::TryRecvError::Closed) => {
                panic!("composer draft load-start probe closed before observation")
            }
        }
    }

    pub fn release_save(&mut self) {
        self.save_release
            .take()
            .expect("composer draft save release is single-use")
            .send(())
            .expect("composer draft save must still be blocked");
    }

    pub async fn wait_for_save_completed(&mut self) {
        (&mut self.save_completed)
            .await
            .expect("composer draft save-completion probe must remain available");
    }

    pub async fn wait_for_load_started(&mut self) {
        (&mut self.load_started)
            .await
            .expect("composer draft load-start probe must remain available");
    }

    pub async fn wait_for_load_completed(&mut self) {
        (&mut self.load_completed)
            .await
            .expect("composer draft load-completion probe must remain available");
    }
}

fn initial_send_read_receipts(state: &AppState) -> bool {
    state.settings.values.notifications.send_read_receipts
}

impl CoreRuntime {
    /// Start the runtime. Must be called within an async runtime context.
    pub fn start() -> Self {
        Self::start_with_data_dir(default_data_dir())
    }

    /// Start with a custom data directory (used by QA binaries and tests).
    pub fn start_with_data_dir(data_dir: PathBuf) -> Self {
        let account_store_actor = StoreActor::new(data_dir.clone());
        let composer_draft_store_actor = StoreActor::new(data_dir.clone());
        #[cfg(any(test, feature = "test-hooks"))]
        let native_artifacts: Arc<dyn NativeArtifactPort> =
            Arc::new(crate::native_artifact::NativeArtifactRegistry::new());
        #[cfg(not(any(test, feature = "test-hooks")))]
        let native_artifacts: Arc<dyn NativeArtifactPort> = Arc::new(RejectingNativeArtifactPort);
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
            native_artifacts,
        )
    }

    /// Start with a custom data directory and injected native artifact port.
    pub fn start_with_data_dir_and_native_artifact_port(
        data_dir: PathBuf,
        native_artifacts: Arc<dyn NativeArtifactPort>,
    ) -> Self {
        let account_store_actor = StoreActor::new(data_dir.clone());
        let composer_draft_store_actor = StoreActor::new(data_dir.clone());
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
            native_artifacts,
        )
    }

    /// Start with a custom data directory and an injected OS credential store
    /// backend. Used by the production Tauri binary to inject the real keyring
    /// adapter (`KeyringCredentialBackend`).
    pub fn start_with_data_dir_and_os_backend(
        data_dir: PathBuf,
        os_backend: std::sync::Arc<dyn koushi_key::CredentialBackend>,
    ) -> Self {
        // The OS-backed actor owns the in-memory credential-vault cache.  Keep one
        // instance per runtime and clone it for the independent consumers so a
        // single launch never asks Keychain for the vault master key twice.
        let account_store_actor = StoreActor::with_os_backend(data_dir.clone(), os_backend);
        let composer_draft_store_actor = account_store_actor.clone();
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
            Arc::new(RejectingNativeArtifactPort),
        )
    }

    /// Start with an injected native artifact path port.
    pub fn start_with_data_dir_and_os_backend_and_native_artifact_port(
        data_dir: PathBuf,
        os_backend: std::sync::Arc<dyn koushi_key::CredentialBackend>,
        native_artifacts: Arc<dyn NativeArtifactPort>,
    ) -> Self {
        let account_store_actor = StoreActor::with_os_backend(data_dir.clone(), os_backend);
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor.clone(),
            account_store_actor,
            native_artifacts,
        )
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn start_with_event_capacity(event_capacity: usize) -> Self {
        let data_dir = default_data_dir();
        let account_store_actor = StoreActor::new(data_dir.clone());
        let composer_draft_store_actor = StoreActor::new(data_dir.clone());
        Self::start_inner(
            event_capacity,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
            Arc::new(RejectingNativeArtifactPort),
        )
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn start_with_data_dir_and_file_credentials(
        data_dir: PathBuf,
        credential_dir: PathBuf,
    ) -> Self {
        let account_store_actor = StoreActor::with_backend(
            koushi_store::CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(
                credential_dir.clone(),
            )),
            data_dir.clone(),
        );
        let composer_draft_store_actor = StoreActor::with_backend(
            koushi_store::CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(
                credential_dir,
            )),
            data_dir.clone(),
        );
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
            Arc::new(crate::native_artifact::NativeArtifactRegistry::new()),
        )
    }

    fn start_inner(
        event_capacity: usize,
        data_dir: PathBuf,
        store_actor: StoreActor,
        composer_draft_store_actor: StoreActor,
        native_artifacts: Arc<dyn NativeArtifactPort>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel(COMMAND_INBOX_CAPACITY);
        // NOTE: action_tx is the high-volume action-projection inbox; it must be
        // ACTION_QUEUE_CAPACITY (not COMMAND_INBOX_CAPACITY) so large-account
        // sync bursts never overflow the RoomActor's drop-on-full try_send.
        let (event_tx, _) = broadcast::channel(event_capacity);
        let (action_tx, action_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        let (event_navigation_prepared_tx, event_navigation_prepared_rx) =
            mpsc::unbounded_channel();
        #[cfg(any(test, feature = "test-hooks"))]
        let (composer_draft_test_tx, composer_draft_test_rx) = mpsc::channel(1);
        let settings_store = SettingsStore::new(&data_dir);
        let composer_draft_leases = Arc::new(ComposerDraftLeaseRegistry::new());
        let sliding_sync_diagnostics = crate::SlidingSyncDiagnostics::default();
        let composer_draft_lease_changes = composer_draft_leases.subscribe();
        let (composer_draft_rejected_tx, composer_draft_rejected_rx) = mpsc::unbounded_channel();

        let mut initial_state = AppState::default();
        let (settings_action, settings_load_status) = match settings_store.load() {
            Ok(values) => (
                AppAction::SettingsLoaded { values },
                SettingsLoadStatus::Loaded,
            ),
            Err(_) => (
                AppAction::SettingsLoadFailed {
                    message: "settings could not be loaded".to_owned(),
                },
                SettingsLoadStatus::Failed,
            ),
        };
        let _ = reduce(&mut initial_state, settings_action);
        let (snapshot_tx, snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
            generation: 0,
            state: initial_state.clone(),
        });

        // Spawn AccountActor with shared channels.
        let account_actor =
            crate::account::AccountActor::spawn_with_diagnostics_and_native_artifacts(
                store_actor,
                action_tx.clone(),
                event_tx.clone(),
                crate::link_preview::LinkPreviewContext::from_settings(
                    &initial_state.settings.values,
                ),
                Arc::clone(&composer_draft_leases),
                initial_send_read_receipts(&initial_state),
                sliding_sync_diagnostics.clone(),
                Arc::clone(&native_artifacts),
            );

        let focused_projection_rx = account_actor
            .take_focused_projection_commits()
            .expect("AppActor must own the focused projection commit receiver");
        #[cfg(any(test, feature = "test-hooks"))]
        let account_actor_test_handle = account_actor.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let composer_draft_store_actor_for_testing = composer_draft_store_actor.clone();
        let actor = AppActor {
            command_rx,
            action_rx,
            event_navigation_prepared_tx,
            event_navigation_prepared_rx,
            pending_event_navigation: None,
            event_navigation_generation: 0,
            event_navigation_task: None,
            event_navigation_deadline_task: None,
            focused_projection_rx: Some(focused_projection_rx),
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_test_rx,
            event_tx: event_tx.clone(),
            snapshot_tx,
            state: initial_state,
            settings_store,
            settings_load_status,
            composer_draft_store_actor,
            composer_draft_load_status: ComposerDraftLoadStatus::Unloaded,
            composer_draft_reload_required: false,
            navigation_loaded_for: None,
            navigation_persistence_status: NavigationPersistenceStatus::Unloaded,
            scheduled_sends_loaded_for: None,
            room_preferences_loaded_for: None,
            state_generation: 0,
            pending_composer_draft_persist: None,
            composer_draft_leases: Arc::clone(&composer_draft_leases),
            composer_draft_lease_changes,
            composer_draft_rejected_tx,
            composer_draft_rejected_rx,
            pending_composer_acceptances: HashMap::new(),
            pending_command_admissions: Vec::new(),
            account_actor,
            activity_projection: ActivityProjection::default(),
            activity_resolution_generation: 0,
            next_internal_request_sequence: 1,
            navigation_projection_generation: 0,
            pending_select: HashMap::new(),
            pending_focused_navigation: None,
            latest_focused_projection_generation: HashMap::new(),
            pending_date_navigation_request_id: None,
        };
        let actor = executor::spawn(actor.run());
        let media_preparation =
            Arc::new(crate::media_preparation::MediaPreparationService::default());
        let media_staging = Arc::new(crate::media_staging::MediaStagingService::new(Arc::clone(
            &media_preparation,
        )));
        let media_preparation_for_lifecycle = Arc::clone(&media_preparation);
        let mut media_snapshot_rx = snapshot_rx.clone();
        let media_lifecycle = executor::spawn(async move {
            loop {
                let snapshot = media_snapshot_rx.borrow().state.clone();
                media_preparation_for_lifecycle
                    .reconcile_snapshot(&snapshot)
                    .await;
                if media_snapshot_rx.changed().await.is_err() {
                    break;
                }
            }
        });

        Self {
            command_tx,
            event_tx,
            snapshot_rx,
            next_connection_id: AtomicU64::new(1),
            composer_draft_leases,
            sliding_sync_diagnostics,
            native_artifacts,
            action_tx,
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_test_tx,
            media_preparation,
            media_staging,
            media_lifecycle: AbortOnDrop::new(media_lifecycle),
            #[cfg(any(test, feature = "test-hooks"))]
            account_actor_test_handle,
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_store_actor_for_testing,
            actor: AbortOnDrop::new(actor),
        }
    }

    pub fn media_preparation(&self) -> &crate::media_preparation::MediaPreparationService {
        &self.media_preparation
    }

    pub fn media_staging(&self) -> &crate::media_staging::MediaStagingService {
        &self.media_staging
    }

    pub fn sliding_sync_diagnostics(&self) -> crate::SlidingSyncDiagnosticsSnapshot {
        self.sliding_sync_diagnostics.snapshot()
    }

    /// Test hook: inject reducer actions as if an actor side effect produced
    /// them. Not part of the public production API.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn inject_actions(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.send(actions).await;
    }

    /// Test hook: inject one typed persisted-draft mutation and wait until the
    /// AppActor has reduced, lifecycle-reconciled, and persisted it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn inject_composer_drafts_and_wait_for_testing(
        &self,
        drafts: ComposerDraftStore,
    ) -> AppState {
        let (completion_tx, completion_rx) = oneshot::channel();
        self.composer_draft_test_tx
            .send(ComposerDraftTestMutation {
                drafts,
                completion: completion_tx,
            })
            .await
            .expect("AppActor composer draft test hook must remain available");
        self.action_tx
            .send(Vec::new())
            .await
            .expect("AppActor action inbox must remain available");
        completion_rx
            .await
            .expect("AppActor composer draft test hook must acknowledge completion")
    }

    /// Test hook: retain the lease registry across runtime shutdown so tests
    /// can prove the account-owned teardown barrier retired the live renderer.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn composer_draft_lease_registry_for_testing(&self) -> Arc<ComposerDraftLeaseRegistry> {
        Arc::clone(&self.composer_draft_leases)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_composer_draft_persistence_permit_for_testing(&self) {
        self.composer_draft_leases
            .fail_next_persistence_permit_for_testing();
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn install_composer_draft_io_barrier_for_testing(
        &self,
    ) -> ComposerDraftIoBarrierForTesting {
        let (save_started_tx, save_started) = oneshot::channel();
        let (save_release, save_release_rx) = std::sync::mpsc::channel();
        let (save_completed_tx, save_completed) = oneshot::channel();
        let (load_started_tx, load_started) = oneshot::channel();
        let (load_completed_tx, load_completed) = oneshot::channel();
        let load_attempt_count = Arc::new(AtomicUsize::new(0));
        self.composer_draft_store_actor_for_testing
            .install_composer_draft_io_probe(
                save_started_tx,
                save_release_rx,
                save_completed_tx,
                load_started_tx,
                load_completed_tx,
                Arc::clone(&load_attempt_count),
            );
        ComposerDraftIoBarrierForTesting {
            save_started,
            save_release: Some(save_release),
            save_completed,
            load_started,
            load_completed,
            load_attempt_count,
        }
    }

    /// Test hook: override current-device trust observation through the typed
    /// AccountActor path. Not part of the public production API.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn configure_trust_observation_for_testing(
        &self,
        observation: koushi_sdk::CurrentDeviceTrustObservation,
    ) -> bool {
        self.account_actor_test_handle
            .send(AccountMessage::ConfigureTrustObservation { observation })
            .await
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn inspect_sync_owners_for_testing(&self) -> (bool, bool, bool) {
        let (response, receiver) = oneshot::channel();
        assert!(
            self.account_actor_test_handle
                .send(AccountMessage::InspectSyncOwners { response })
                .await
        );
        receiver.await.expect("sync owner inspection response")
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn set_current_device_trust_for_testing(
        &self,
        trust: koushi_state::CurrentDeviceTrustState,
    ) -> bool {
        self.account_actor_test_handle
            .send(AccountMessage::SetCurrentDeviceTrustForTesting { trust })
            .await
    }

    pub fn shutdown_handle(&self) -> &executor::JoinHandle<()> {
        self.actor.get()
    }

    /// Close the command inbox and wait until AppActor has completed its
    /// ordered AccountActor/store shutdown barrier.
    pub async fn shutdown(self) {
        let Self {
            command_tx,
            event_tx: _,
            snapshot_rx: _,
            next_connection_id: _,
            composer_draft_leases: _,
            sliding_sync_diagnostics: _,
            native_artifacts: _,
            action_tx: _,
            #[cfg(any(test, feature = "test-hooks"))]
                composer_draft_test_tx: _,
            media_preparation: _,
            media_staging: _,
            mut media_lifecycle,
            #[cfg(any(test, feature = "test-hooks"))]
                account_actor_test_handle: _,
            #[cfg(any(test, feature = "test-hooks"))]
                composer_draft_store_actor_for_testing: _,
            mut actor,
        } = self;
        drop(command_tx);
        let _ = actor.take().await;
        let _ = media_lifecycle.take().await;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsLoadStatus {
    Loaded,
    Failed,
}

struct AppActor {
    command_rx: mpsc::Receiver<CoreCommandEnvelope>,
    action_rx: mpsc::Receiver<Vec<AppAction>>,
    event_navigation_prepared_tx: mpsc::UnboundedSender<EventNavigationPrepared>,
    event_navigation_prepared_rx: mpsc::UnboundedReceiver<EventNavigationPrepared>,
    pending_event_navigation: Option<PendingEventNavigation>,
    event_navigation_generation: u64,
    event_navigation_task: Option<AbortOnDrop<()>>,
    event_navigation_deadline_task: Option<AbortOnDrop<()>>,
    focused_projection_rx:
        Option<mpsc::UnboundedReceiver<crate::timeline::FocusedProjectionCommitted>>,
    #[cfg(any(test, feature = "test-hooks"))]
    composer_draft_test_rx: mpsc::Receiver<ComposerDraftTestMutation>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_tx: watch::Sender<VersionedAppStateSnapshot>,
    state: AppState,
    settings_store: SettingsStore,
    settings_load_status: SettingsLoadStatus,
    composer_draft_store_actor: StoreActor,
    composer_draft_load_status: ComposerDraftLoadStatus,
    /// A lock/unlock can retain the same account key but still requires a
    /// fresh draft load after any captured pre-transition save is flushed.
    composer_draft_reload_required: bool,
    navigation_loaded_for: Option<koushi_protocol::SessionKeyId>,
    navigation_persistence_status: NavigationPersistenceStatus,
    scheduled_sends_loaded_for: Option<koushi_protocol::SessionKeyId>,
    room_preferences_loaded_for: Option<koushi_protocol::SessionKeyId>,
    state_generation: u64,
    pending_composer_draft_persist: Option<PendingComposerDraftPersist>,
    composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    composer_draft_lease_changes: watch::Receiver<()>,
    composer_draft_rejected_tx: mpsc::UnboundedSender<RequestId>,
    composer_draft_rejected_rx: mpsc::UnboundedReceiver<RequestId>,
    pending_composer_acceptances: HashMap<RequestId, PendingComposerAcceptance>,
    pending_command_admissions: Vec<oneshot::Sender<CoreCommandAdmission>>,
    account_actor: AccountActorHandle,
    activity_projection: ActivityProjection,
    activity_resolution_generation: u64,
    next_internal_request_sequence: u64,
    /// Private ordering fence for committed room projections. Request ids are
    /// correlation values and are not monotonic across connections.
    navigation_projection_generation: u64,
    /// Correlation map for SelectRoom intents: room_id → FIFO queue of request_ids.
    /// Multiple concurrent SelectRoom commands for the same room are queued in
    /// submission order; each `AppAction::SelectRoom` pops the oldest entry so
    /// every submitted command receives a terminal `IntentLifecycle` outcome.
    /// Private-data-free: stores opaque ids only, never room names or content.
    pending_select: HashMap<String, std::collections::VecDeque<RequestId>>,
    /// Main-pane Focused navigation awaiting proof that the WebView canonical
    /// store applied the actor-owned InitialItems projection.
    pending_focused_navigation: Option<PendingFocusedNavigation>,
    latest_focused_projection_generation: HashMap<TimelineKey, (u64, TimelineGeneration)>,
    pending_date_navigation_request_id: Option<RequestId>,
}

enum CommandDisposition {
    Handle(CoreCommandEnvelope),
    Shutdown,
}

fn command_disposition(envelope: CoreCommandEnvelope) -> CommandDisposition {
    if matches!(
        &envelope,
        CoreCommandEnvelope::Public {
            command: CoreCommand::App(AppCommand::Shutdown { .. }),
            ..
        }
    ) {
        CommandDisposition::Shutdown
    } else {
        CommandDisposition::Handle(envelope)
    }
}

async fn receive_focused_projection_commit(
    receiver: &mut Option<mpsc::UnboundedReceiver<crate::timeline::FocusedProjectionCommitted>>,
) -> Option<crate::timeline::FocusedProjectionCommitted> {
    let Some(active) = receiver.as_mut() else {
        return future::pending().await;
    };
    match active.recv().await {
        Some(commit) => Some(commit),
        None => {
            *receiver = None;
            None
        }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
struct ComposerDraftTestMutation {
    drafts: ComposerDraftStore,
    completion: oneshot::Sender<AppState>,
}

impl AppActor {
    /// Issue #450: the TimelineKey of the composer target a slash-command
    /// rejection should be keyed to (canonical user id account key).
    fn composer_target_notice_key(
        &self,
        account: &koushi_protocol::SessionKeyId,
        target: &ComposerTarget,
    ) -> Option<TimelineKey> {
        let account_key = koushi_protocol::ids::AccountKey(account.user_id.clone());
        match target {
            ComposerTarget::Main { room_id } => {
                Some(TimelineKey::room(account_key, room_id.clone()))
            }
            ComposerTarget::Thread {
                room_id,
                root_event_id,
            } => Some(TimelineKey {
                account_key,
                kind: koushi_protocol::ids::TimelineKind::Thread {
                    room_id: room_id.clone(),
                    root_event_id: root_event_id.clone(),
                },
            }),
        }
    }

    async fn run(mut self) {
        loop {
            let composer_draft_persist_delay = self.composer_draft_persist_delay();
            let scheduled_send_delay = self.scheduled_send_delay();
            tokio::select! {
                _ = async {
                    match composer_draft_persist_delay {
                        Some(delay) => executor::sleep(delay).await,
                        None => future::pending::<()>().await,
                    }
                } => {
                    self.flush_pending_composer_drafts().await;
                }
                _ = async {
                    match scheduled_send_delay {
                        Some(delay) => executor::sleep(delay).await,
                        None => future::pending::<()>().await,
                    }
                } => {
                    let before_state = self.state.clone();
                    if self.dispatch_due_scheduled_send().await {
                        self.publish_state_change(&before_state);
                    }
                }
                lease_change = self.composer_draft_lease_changes.changed() => {
                    if lease_change.is_err() {
                        break;
                    }
                    let before_state = self.state.clone();
                    if self
                        .reconcile_composer_draft_lifecycle_after_permit_change()
                        .await
                    {
                        self.publish_state_change(&before_state);
                    }
                }
                rejected_request_id = self.composer_draft_rejected_rx.recv() => {
                    let Some(rejected_request_id) = rejected_request_id else {
                        break;
                    };
                    self.pending_composer_acceptances.remove(&rejected_request_id);
                }
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break };
                    let loop_started = std::time::Instant::now();
                    let _clone_probe = self.state.clone();
                    let clone_ms = loop_started.elapsed().as_millis();
                    let mut state_changed = match command_disposition(command) {
                        CommandDisposition::Handle(command) => self.handle_command(command).await,
                        CommandDisposition::Shutdown => break,
                    };
                    let mut handled = 1u32;
                    let mut shutdown = false;
                    // Coalesce: drain whatever is already queued before
                    // emitting a single StateDelta for the batch. Shutdown is
                    // an ordered barrier: publish preceding changes, then stop
                    // without handling duplicate or later commands.
                    while let Ok(next) = self.command_rx.try_recv() {
                        match command_disposition(next) {
                            CommandDisposition::Handle(next) => {
                                state_changed |= self.handle_command(next).await;
                                handled += 1;
                            }
                            CommandDisposition::Shutdown => {
                                shutdown = true;
                                break;
                            }
                        }
                    }
                    if state_changed {
                        // A command arm may have published an intent commit point
                        // already. Diff only from the latest published watch value so
                        // coalescing cannot duplicate that delta or skip later commands.
                        let published_state = self.snapshot_tx.borrow().state.clone();
                        self.publish_state_change(&published_state);
                    }
                    self.settle_command_admissions();
                    app_loop_trace("command", handled, clone_ms, loop_started.elapsed());
                    if shutdown {
                        break;
                    }
                }
                event_navigation_prepared = self.event_navigation_prepared_rx.recv() => {
                    if let Some(event_navigation_prepared) = event_navigation_prepared {
                        self.handle_event_navigation_prepared(event_navigation_prepared).await;
                    } else {
                        break;
                    }
                }
                focused_projection = receive_focused_projection_commit(
                    &mut self.focused_projection_rx
                ) => {
                    if let Some(focused_projection) = focused_projection {
                        self.handle_focused_projection_commit(focused_projection).await;
                    }
                }
                actions = self.action_rx.recv() => {
                    let Some(actions) = actions else { break };
                    #[cfg(any(test, feature = "test-hooks"))]
                    let composer_draft_test_completions =
                        self.apply_pending_composer_draft_test_mutations().await;
                    let loop_started = std::time::Instant::now();
                    let action_batch = actions.len() as u32;
                    let before_state = self.state.clone();
                    let clone_ms = loop_started.elapsed().as_millis();
                    let mut state_changed = false;
                    let mut pending_select_settlements = Vec::new();
                    let mut post_projection_work: Vec<(
                        Vec<AppEffect>,
                        Option<u64>,
                        Option<RequestId>,
                        crate::timeline::NavigationProjectionCleanup,
                        reducer_support::DeferredReducerSideEffects,
                        Option<ComposerAcceptanceIdentity>,
                    )> = Vec::new();
                    for action in actions {
                        let Some(action) = normalize_activity_resolution_action(&self.state, action)
                        else {
                            continue;
                        };
                        // Load each session-owned view before later projections can
                        // mutate it, unless an earlier action in this batch captured a
                        // persistence fence that must be applied first post-commit.
                        let navigation_load_fenced = post_projection_work.iter().any(
                            |(_, _, _, _, deferred, _)| deferred.has_navigation_persist(),
                        );
                        let composer_load_fenced = post_projection_work.iter().any(
                            |(_, _, _, _, deferred, _)| deferred.has_composer_draft_persist(),
                        );
                        let scheduled_load_fenced = post_projection_work.iter().any(
                            |(_, _, _, _, deferred, _)| deferred.has_scheduled_send_persist(),
                        );
                        if !navigation_load_fenced
                            && !matches!(&action, AppAction::NavigationLoaded { .. })
                        {
                            self.load_navigation_for_current_session().await;
                        }
                        if !composer_load_fenced {
                            self.load_composer_drafts_for_current_session().await;
                        }
                        if !scheduled_load_fenced {
                            self.load_scheduled_sends_for_current_session().await;
                        }
                        let action = guard_activity_resolution_completion(&self.state, action);
                        let composer_acceptance =
                            composer_acceptance_identity_for_action(&action);
                        let trust_projection_transition = match &action {
                            AppAction::AuthoritativeDeviceTrustChanged { generation, transition_id, .. } => {
                                Some((*generation, *transition_id))
                            }
                            _ => None,
                        };
                        match &action {
                            AppAction::ActivityRowsObserved { rows } => {
                                self.activity_projection.ingest(rows.clone());
                            }
                            AppAction::CanonicalActivityWindowReconciled {
                                room_id,
                                rows,
                                redacted_event_ids,
                                hidden_event_ids,
                            } => {
                                self.activity_projection.reconcile_canonical_window(
                                    room_id.clone(),
                                    rows.clone(),
                                    redacted_event_ids.clone(),
                                    hidden_event_ids.clone(),
                                );
                            }
                            AppAction::ActivityResolutionRowsObserved { rows, .. } => {
                                self.activity_projection.ingest_resolution_rows(rows.clone());
                            }
                            _ => {}
                        }
                        if let (
                            Some(projection_request_id),
                            AppAction::OpenFocusedContext { room_id, event_id },
                        ) = (self.pending_date_navigation_request_id, &action)
                        {
                            if let Some(account_key) = self.current_account_key() {
                                self.pending_focused_navigation = Some(PendingFocusedNavigation {
                                    projection_request_id,
                                    key: TimelineKey {
                                        account_key,
                                        kind: TimelineKind::Focused {
                                            room_id: room_id.clone(),
                                            event_id: event_id.clone(),
                                        },
                                    },
                                    room_id: room_id.clone(),
                                    event_id: event_id.clone(),
                                    allow_live_fallback: true,
                                    generation: None,
                                });
                            }
                        }
                        if self.pending_date_navigation_request_id.is_some()
                            && matches!(&action, AppAction::EnterAnchoredTimeline { .. })
                        {
                            // The account actor emits the legacy pair atomically;
                            // retain only Open and wait for the WebView projection ACK.
                            self.pending_date_navigation_request_id = None;
                            continue;
                        }
                        // For SelectRoom: capture observable facts BEFORE reduce so
                        // we can classify the outcome afterwards and emit the
                        // telemetry-lane IntentLifecycle event. Private-data-free:
                        // we capture only boolean flags and a count.
                        let select_intent_pre: Option<(String, bool, bool, bool, usize)> =
                            if let AppAction::SelectRoom { room_id } = &action {
                                let session_ready = matches!(
                                    self.state.session,
                                    SessionState::Ready(_)
                                );
                                let found =
                                    self.state.rooms.iter().any(|r| r.room_id == *room_id);
                                let already = self
                                    .state
                                    .navigation
                                    .active_room_id
                                    .as_deref()
                                    == Some(room_id.as_str());
                                let rooms_len = self.state.rooms.len();
                                Some((
                                    room_id.clone(),
                                    session_ready,
                                    found,
                                    already,
                                    rooms_len,
                                ))
                            } else {
                                None
                            };
                        // Actor-originated actions are post-side-effect
                        // projections: the owner actor has already performed
                        // the corresponding Matrix/store/sync operation.
                        // AppActor owns AppCommand effects above; replaying
                        // actor-projection effects here would double-execute
                        // login, restore, sync, or recovery work.
                        let active_room_before_reduce =
                            self.state.navigation.active_room_id.clone();
                        let room_timeline_before_reduce = self.current_room_timeline_key();
                        let action_for_navigation_cleanup = action.clone();
                        let (post_projection_effects, deferred_reducer_side_effects) =
                            self.reduce_app_action_state(action);
                        if deferred_reducer_side_effects.discards_composer_drafts() {
                            // A destructive transition in this same reducer batch
                            // supersedes draft saves captured by earlier actions.
                            for (_, _, _, _, queued_deferred, _) in &mut post_projection_work {
                                queued_deferred.cancel_composer_draft_persist();
                            }
                        }
                        let active_room_changed = active_room_before_reduce
                            != self.state.navigation.active_room_id;
                        let replacement_room_for_cleanup = navigation_replacement_room_for_cleanup(
                            &action_for_navigation_cleanup,
                            active_room_before_reduce.as_deref(),
                            self.state.navigation.active_room_id.as_deref(),
                        );
                        let replacement_room_id_for_cleanup = replacement_room_for_cleanup
                            .as_ref()
                            .and_then(NavigationReplacementRoomForCleanup::room_id);
                        let cancel_replaced_room_timeline_pagination =
                            replacement_room_for_cleanup.as_ref().and_then(|_| {
                                cancel_replaced_room_timeline_pagination_key(
                                    room_timeline_before_reduce.clone(),
                                    replacement_room_id_for_cleanup,
                                )
                            });
                        let cancel_replaced_room_timeline_link_previews =
                            replacement_room_for_cleanup.as_ref().and_then(|_| {
                                cancel_replaced_room_timeline_link_previews_key(
                                    room_timeline_before_reduce.clone(),
                                    replacement_room_id_for_cleanup,
                                )
                            });
                        let navigation_projection_generation = if active_room_changed {
                            match self.navigation_projection_generation.checked_add(1) {
                                Some(generation) => {
                                    self.navigation_projection_generation = generation;
                                    Some(generation)
                                }
                                None => {
                                    record(DiagnosticEvent::new(
                                        DiagnosticLevel::Error,
                                        "core.navigation",
                                        "projection_generation_exhausted",
                                    ));
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        if matches!(
                            action_for_navigation_cleanup,
                            AppAction::SelectSpace { .. }
                        ) {
                            record(
                                DiagnosticEvent::new(
                                    DiagnosticLevel::Debug,
                                    "core.space.transition",
                                    "reduce",
                                )
                                .field(DiagnosticField::boolean(
                                    "active_room_changed",
                                    active_room_changed,
                                ))
                                .field(DiagnosticField::boolean(
                                    "active_room_present",
                                    self.state.navigation.active_room_id.is_some(),
                                ))
                                .field(DiagnosticField::boolean(
                                    "cleanup_pending",
                                    cancel_replaced_room_timeline_pagination.is_some()
                                        || cancel_replaced_room_timeline_link_previews.is_some(),
                                ))
                                .field(DiagnosticField::count(
                                    "rooms",
                                    self.state.rooms.len() as u64,
                                ))
                                .field(DiagnosticField::count(
                                    "projection_generation",
                                    navigation_projection_generation.unwrap_or(0),
                                )),
                            );
                        }
                        let mut navigation_projection_cause = None;
                        if let Some((generation, transition_id)) = trust_projection_transition {
                            let ready = matches!(self.state.session, SessionState::Ready(_));
                            let locked = matches!(self.state.session, SessionState::Locked(_));
                            record(
                                DiagnosticEvent::new(
                                    DiagnosticLevel::Info,
                                    "core.verification_admission",
                                    if ready {
                                        "trust_projection_reduced_ready"
                                    } else if locked {
                                        "trust_projection_reduced_locked"
                                    } else {
                                        "trust_projection_reduced_gated"
                                    },
                                )
                                .field(DiagnosticField::count("generation", generation))
                                .field(DiagnosticField::count("transition_id", transition_id)),
                            );
                            let delivered = self
                                .account_actor
                                .send(AccountMessage::TrustProjectionApplied {
                                    generation,
                                    transition_id,
                                    ready,
                                    locked,
                                })
                                .await;
                            record(
                                DiagnosticEvent::new(
                                    if delivered {
                                        DiagnosticLevel::Info
                                    } else {
                                        DiagnosticLevel::Warn
                                    },
                                    "core.verification_admission",
                                    if delivered {
                                        "trust_projection_ack_delivered"
                                    } else {
                                        "trust_projection_ack_delivery_failed"
                                    },
                                )
                                .field(DiagnosticField::count("generation", generation))
                                .field(DiagnosticField::count("transition_id", transition_id)),
                            );
                        }
                        // Capture the correlated request now, but settle its outcome after
                        // the whole action batch has reduced. This keeps telemetry before
                        // publication while ensuring a superseded intermediate room is not
                        // reported as committed.
                        let select_request_id = select_intent_pre.as_ref().and_then(
                            |(room_id, ..)| {
                                let request_id = self
                                    .pending_select
                                    .get_mut(room_id)
                                    .and_then(|q| q.pop_front());
                                if self
                                    .pending_select
                                    .get(room_id)
                                    .map(|q| q.is_empty())
                                    .unwrap_or(false)
                                {
                                    self.pending_select.remove(room_id);
                                }
                                request_id
                            },
                        );
                        if let Some((room_id, session_ready, found, already, rooms_len)) =
                            select_intent_pre
                        {
                            let committed = self
                                .state
                                .navigation
                                .active_room_id
                                .as_deref()
                                == Some(room_id.as_str());
                            record(
                                DiagnosticEvent::new(
                                    DiagnosticLevel::Debug,
                                    "core.intent",
                                    "select_reduce",
                                )
                                .field(DiagnosticField::boolean("found", found))
                                .field(DiagnosticField::boolean("session_ready", session_ready))
                                .field(DiagnosticField::count("rooms", rooms_len as u64))
                                .field(DiagnosticField::boolean("committed", committed)),
                            );
                            if let Some(request_id) = select_request_id {
                                pending_select_settlements.push((
                                    request_id,
                                    room_id,
                                    session_ready,
                                    found,
                                    already,
                                    rooms_len,
                                    committed,
                                ));
                            }
                            if committed {
                                navigation_projection_cause = select_request_id;
                            }
                        }
                        // Stage 1 keeps only synchronous state derivation before publish.
                        // Every transport, cleanup, persistence, and UI effect from this
                        // action is queued uniformly for the post-commit stage below.
                        if let Some(activity_update) = self
                            .activity_projection
                            .update_action_for_open_state(&self.state)
                        {
                            let (activity_effects, activity_deferred) =
                                self.reduce_app_action_state(activity_update);
                            post_projection_work.push((
                                activity_effects,
                                None,
                                None,
                                crate::timeline::NavigationProjectionCleanup::default(),
                                activity_deferred,
                                None,
                            ));
                        }
                        post_projection_work.push((
                            post_projection_effects,
                            navigation_projection_generation,
                            navigation_projection_cause,
                            crate::timeline::NavigationProjectionCleanup {
                                cancel_pagination: cancel_replaced_room_timeline_pagination,
                                cancel_link_previews: cancel_replaced_room_timeline_link_previews,
                            },
                            deferred_reducer_side_effects,
                            composer_acceptance,
                        ));
                        state_changed = true;
                    }
                    let published_generation = if state_changed {
                        self.publish_state_change(&before_state)
                    } else {
                        self.state_generation
                    };
                    let final_active_room_id = self.state.navigation.active_room_id.clone();
                    for (
                        request_id,
                        room_id,
                        session_ready,
                        found,
                        already,
                        rooms_len,
                        reduced_committed,
                    ) in pending_select_settlements
                    {
                        let outcome = if !session_ready {
                            IntentOutcome::FailedNoOp(IntentNoOpReason::SessionNotReady)
                        } else if !found {
                            IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState)
                        } else if (already || reduced_committed)
                            && final_active_room_id.as_deref() != Some(room_id.as_str())
                        {
                            IntentOutcome::FailedNoOp(IntentNoOpReason::Superseded)
                        } else if already {
                            IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive)
                        } else if reduced_committed {
                            IntentOutcome::Committed
                        } else {
                            IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState)
                        };
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Debug,
                                "core.intent",
                                "lifecycle",
                            )
                            .field(DiagnosticField::request_id(
                                "request_id",
                                request_id.connection_id.0,
                                request_id.sequence,
                            ))
                            .field(DiagnosticField::count("rooms", rooms_len as u64))
                            .field(DiagnosticField::token(
                                "outcome",
                                intent_outcome_token(&outcome),
                            )),
                        );
                        self.handle_event_navigation_select_outcome(request_id, outcome.clone())
                            .await;
                        self.emit(CoreEvent::IntentLifecycle {
                            request_id,
                            outcome,
                            published_generation,
                        });
                    }

                    // Only after publication and every terminal has been emitted may
                    // cleanup, persistence, and other post-commit effects run.
                    for (
                        effects,
                        navigation_projection_generation,
                        navigation_projection_cause,
                        navigation_cleanup,
                        deferred_reducer_side_effects,
                        composer_acceptance,
                    ) in post_projection_work
                    {
                        let before_post_projection = self.state.clone();
                        self.handle_post_projection_effects(
                            &effects,
                            navigation_projection_generation,
                            navigation_projection_cause,
                            navigation_cleanup,
                        )
                        .await;
                        self.apply_deferred_reducer_side_effects(
                            deferred_reducer_side_effects,
                        )
                        .await;
                        if let Some(identity) = composer_acceptance {
                            self.pending_composer_acceptances
                                .retain(|_, pending| pending.identity != identity);
                        }
                        self.handle_ui_event_effects(&effects).await;
                        if self.state != before_post_projection {
                            self.publish_state_change(&before_post_projection);
                        }
                    }
                    // Apply every captured persistence effect before loading the
                    // final session's views. In particular, an old-account draft
                    // save must not be overtaken by the new-account load.
                    let before_post_commit_loads = self.state.clone();
                    self.load_room_preferences_for_current_session().await;
                    self.load_navigation_for_current_session().await;
                    self.load_composer_drafts_for_current_session().await;
                    self.load_scheduled_sends_for_current_session().await;
                    if self.state != before_post_commit_loads {
                        self.publish_state_change(&before_post_commit_loads);
                    }
                    #[cfg(any(test, feature = "test-hooks"))]
                    for completion in composer_draft_test_completions {
                        let _ = completion.send(self.state.clone());
                    }
                    app_loop_trace("action", action_batch, clone_ms, loop_started.elapsed());
                }
            }
        }
        // Shutdown: tell AccountActor to stop.
        self.flush_pending_composer_drafts().await;
        let _ = self.account_actor.send(AccountMessage::Shutdown).await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    async fn apply_pending_composer_draft_test_mutations(
        &mut self,
    ) -> Vec<oneshot::Sender<AppState>> {
        let mut completions = Vec::new();
        while let Ok(mutation) = self.composer_draft_test_rx.try_recv() {
            self.flush_pending_composer_drafts().await;
            let before_state = self.state.clone();
            let effects = self
                .reduce_app_action(AppAction::ComposerDraftsLoaded {
                    drafts: mutation.drafts,
                })
                .await;
            self.handle_ui_event_effects(&effects).await;
            self.publish_state_delta(&before_state);
            self.flush_pending_composer_drafts().await;
            let before_reconcile = self.state.clone();
            if self
                .reconcile_composer_draft_lifecycle_after_permit_change()
                .await
            {
                self.publish_state_delta(&before_reconcile);
            }
            self.flush_pending_composer_drafts().await;
            completions.push(mutation.completion);
        }
        completions
    }

    async fn load_room_preferences_for_current_session(&mut self) {
        let Some(key_id) = room_preferences_session_key(&self.state) else {
            self.room_preferences_loaded_for = None;
            return;
        };
        if self.room_preferences_loaded_for.as_ref() == Some(&key_id) {
            return;
        }

        let store = self.composer_draft_store_actor.clone();
        let load_key_id = key_id.clone();
        let preferences = executor::spawn_blocking(move || {
            store
                .load_room_preferences(&load_key_id)
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let effects = reduce(
            &mut self.state,
            AppAction::RoomPreferencesLoaded { preferences },
        );
        self.room_preferences_loaded_for = Some(key_id);
        self.handle_ui_event_effects(&effects).await;
    }

    async fn persist_room_preferences(&mut self, preferences: &koushi_state::RoomPreferencesState) {
        let Some(key_id) = room_preferences_session_key(&self.state) else {
            return;
        };
        let store = self.composer_draft_store_actor.clone();
        let preferences = preferences.clone();
        let _ =
            executor::spawn_blocking(move || store.save_room_preferences(&key_id, &preferences))
                .await;
    }

    fn next_internal_request_id(&mut self) -> RequestId {
        let sequence = self.next_internal_request_sequence;
        self.next_internal_request_sequence = self.next_internal_request_sequence.saturating_add(1);
        RequestId {
            connection_id: INTERNAL_RUNTIME_CONNECTION_ID,
            sequence,
        }
    }

    async fn start_activity_resolution(&mut self) {
        let placeholder_room_ids = match &self.state.activity {
            ActivityState::Open { unread, .. } => unread
                .rows
                .iter()
                .filter(|row| row.kind == ActivityRowKind::RoomUnread)
                .map(|row| row.room_id.as_str())
                .collect::<BTreeSet<_>>(),
            _ => return,
        };
        if placeholder_room_ids.is_empty() {
            return;
        }
        let total_unresolved_room_count = placeholder_room_ids.len().try_into().unwrap_or(u32::MAX);
        let requests = self
            .state
            .rooms
            .iter()
            .filter(|room| placeholder_room_ids.contains(room.room_id.as_str()))
            .map(|room| ActivityResolutionRequest {
                room_id: room.room_id.clone(),
                fully_read_event_id: self
                    .state
                    .live_signals
                    .rooms
                    .get(&room.room_id)
                    .and_then(|signals| signals.fully_read_event_id.clone()),
                minimum_unread_count: room.notification_count.max(room.highlight_count).max(1),
            })
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return;
        }
        self.activity_resolution_generation = self.activity_resolution_generation.saturating_add(1);
        let generation = self.activity_resolution_generation;
        let requests = cap_activity_resolution_requests(requests, generation);
        let effects = self
            .reduce_app_action(AppAction::ActivityResolutionStarted {
                generation,
                unresolved_room_count: total_unresolved_room_count,
            })
            .await;
        self.handle_ui_event_effects(&effects).await;
        let _ = self
            .account_actor
            .send(AccountMessage::ResolveActivity {
                generation,
                requests,
            })
            .await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    async fn handle_qa_command(&mut self, command: CoreQaCommand) -> bool {
        match command {
            CoreQaCommand::SetLocalDeviceBlacklisted {
                request_id,
                target,
                room_id,
                acknowledged,
            } => {
                let _ = self
                    .account_actor
                    .send(
                        crate::account::AccountMessage::QaSetLocalDeviceBlacklisted {
                            request_id,
                            target,
                            room_id,
                            acknowledged,
                        },
                    )
                    .await;
            }
            CoreQaCommand::RefreshDeviceKeysAndAssertKnown {
                request_id,
                target,
                acknowledged,
            } => {
                let _ = self
                    .account_actor
                    .send(
                        crate::account::AccountMessage::QaRefreshDeviceKeysAndAssertKnown {
                            request_id,
                            target,
                            acknowledged,
                        },
                    )
                    .await;
            }
            CoreQaCommand::AssertInboundSessionsStartAtZero {
                request_id,
                room_id,
                acknowledged,
            } => {
                let _ = self
                    .account_actor
                    .send(
                        crate::account::AccountMessage::QaAssertInboundSessionsStartAtZero {
                            request_id,
                            room_id,
                            acknowledged,
                        },
                    )
                    .await;
            }
            CoreQaCommand::SyncOnce { request_id } => {
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::QaSyncOnce { request_id })
                    .await;
            }
        }
        false
    }

    /// Returns whether `AppState` changed.
    async fn handle_command(&mut self, envelope: CoreCommandEnvelope) -> bool {
        let (command, composer_permit, admission) = match envelope {
            CoreCommandEnvelope::Public {
                command,
                composer_permit,
                admission,
            } => (command, composer_permit, admission),
            #[cfg(any(test, feature = "test-hooks"))]
            CoreCommandEnvelope::Qa(command) => return self.handle_qa_command(command).await,
        };
        if let Some(admission) = admission {
            self.pending_command_admissions.push(admission);
        }
        debug_assert_eq!(
            command.composer_draft_scope().is_some(),
            composer_permit.is_some(),
            "composer revision commands must enter AppActor with an exact permit"
        );
        let mut composer_permit = composer_permit;
        let command_request_id = command.request_id();
        if command.composer_draft_scope().is_some()
            && !composer_draft_session_key(&self.state).is_some_and(|key_id| {
                matches!(
                    &self.composer_draft_load_status,
                    ComposerDraftLoadStatus::Loaded(loaded_key) if loaded_key == &key_id
                )
            })
        {
            self.emit(CoreEvent::OperationFailed {
                request_id: command_request_id,
                failure: CoreFailure::StoreUnavailable,
            });
            return false;
        }
        if command.requires_ready_session()
            && !is_ready_session_for_commands(&self.state.session)
            && !is_verification_gate_command(&command, &self.state.session)
        {
            trace_runtime_sync!(
                "command_rejected",
                [
                    DiagnosticField::request_id(
                        "request_id",
                        command.request_id().connection_id.0,
                        command.request_id().sequence
                    ),
                    DiagnosticField::token("reason", "session_required"),
                    DiagnosticField::token("action", "emit_operation_failed"),
                ],
                "request_id={} reason=session_required action=emit_operation_failed",
                runtime_request_id_trace_label(command.request_id())
            );
            self.emit(CoreEvent::OperationFailed {
                request_id: command_request_id,
                failure: CoreFailure::SessionRequired,
            });
            if let Some((request_id, kind)) = native_artifact_for_command(&command) {
                self.account_actor
                    .unregister_native_artifact(request_id, kind);
            }
            return false;
        }
        if command_supersedes_event_navigation(&command) {
            self.cancel_event_navigation_owner().await;
        }

        match command {
            CoreCommand::Account(account_command) => {
                if let AccountCommand::LoginPassword { request_id, .. }
                | AccountCommand::CompleteOidcLogin { request_id, .. } = &account_command
                {
                    if !matches!(
                        self.state.session,
                        SessionState::SignedOut | SessionState::Authenticating { .. }
                    ) {
                        self.emit(CoreEvent::OperationFailed {
                            request_id: *request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        return false;
                    }
                }
                let display_label_user_id = match &account_command {
                    AccountCommand::SetLocalUserAlias { user_id, .. } => Some(user_id.as_str()),
                    _ => None,
                };
                let current_session_status_already_checking = matches!(
                    self.state.current_session_status,
                    koushi_state::CurrentSessionStatusState::Checking { .. }
                );
                let display_label_user_ids = display_label_user_id.into_iter().collect::<Vec<_>>();
                let effects =
                    if let Some(action) = account_command_projected_action(&account_command) {
                        self.reduce_app_action(action).await
                    } else {
                        Vec::new()
                    };
                let projected_state_changed = !effects.is_empty();
                if matches!(
                    &account_command,
                    AccountCommand::RefreshCurrentSessionStatus { .. }
                ) {
                    self.handle_app_effects(command_request_id, effects).await;
                    if let Some(event) = current_session_status_noop_event(
                        command_request_id,
                        current_session_status_already_checking,
                        self.state_generation,
                    ) {
                        self.emit(event);
                    }
                    return projected_state_changed;
                }
                self.handle_ui_event_effects_with_display_label_users(
                    &effects,
                    &display_label_user_ids,
                )
                .await;
                let requires_projection_acceptance = matches!(
                    &account_command,
                    AccountCommand::BootstrapSecureBackup { .. }
                        | AccountCommand::RestoreSession { .. }
                        | AccountCommand::RestoreLastSession { .. }
                        | AccountCommand::ResetLocalData { .. }
                        | AccountCommand::StartDeviceCleanup { .. }
                        | AccountCommand::SubmitDeviceCleanupUia { .. }
                        | AccountCommand::EraseDeviceCleanupLocalDataAnyway { .. }
                        | AccountCommand::SubmitRecovery { .. }
                        | AccountCommand::StartSessionBootstrap { .. }
                        | AccountCommand::ConfirmSessionBootstrapSaved { .. }
                        | AccountCommand::StartOwnUserSas { .. }
                );
                let should_route = !requires_projection_acceptance || projected_state_changed;
                if !should_route {
                    let failure =
                        secure_backup_setup_projection_failure(&self.state, &account_command)
                            .unwrap_or(CoreFailure::SessionRequired);
                    self.emit(CoreEvent::OperationFailed {
                        request_id: command_request_id,
                        failure,
                    });
                    if let Some((request_id, kind)) =
                        native_artifact_for_account_command(&account_command)
                    {
                        self.account_actor
                            .unregister_native_artifact(request_id, kind);
                    }
                    return false;
                }
                // Route to AccountActor; it will produce AppActions and
                // CoreEvents. AppActor does not immediately know the result —
                // it observes it via the action channel.
                let native_artifact = native_artifact_for_account_command(&account_command);
                let sent = self
                    .account_actor
                    .send(AccountMessage::Command(account_command))
                    .await;
                if !sent {
                    if let Some((request_id, kind)) = native_artifact {
                        self.account_actor
                            .unregister_native_artifact(request_id, kind);
                    }
                }
                projected_state_changed
            }
            CoreCommand::App(app_command) => match app_command {
                AppCommand::NavigateToEvent {
                    request_id,
                    room_id,
                    event_id,
                    source,
                    missing_target_policy,
                } => {
                    self.handle_event_navigation_command(
                        request_id,
                        room_id,
                        event_id,
                        source,
                        missing_target_policy,
                    )
                    .await
                }
                AppCommand::Shutdown { .. } => {
                    unreachable!("shutdown is handled by the AppActor command disposition")
                }
                AppCommand::SetComposerReplyTarget {
                    request_id,
                    room_id,
                    event_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::ComposerReplyTargetSelected {
                            room_id,
                            event_id,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CancelComposerReply { request_id } => {
                    let effects = self
                        .reduce_app_action(AppAction::ComposerReplyCancelled)
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetComposerDraft {
                    request_id,
                    expected_account,
                    room_id,
                    document,
                    revision,
                } => {
                    if !composer_draft_account_matches(&self.state, &expected_account) {
                        return false;
                    }
                    let effects = self
                        .reduce_app_action(AppAction::ComposerDraftChangedAtRevision {
                            room_id,
                            document,
                            revision,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetThreadComposerDraft {
                    request_id,
                    expected_account,
                    room_id,
                    root_event_id,
                    document,
                    revision,
                } => {
                    if !composer_draft_account_matches(&self.state, &expected_account) {
                        return false;
                    }
                    let effects = self
                        .reduce_app_action(AppAction::ThreadComposerDraftChangedAtRevision {
                            room_id,
                            root_event_id,
                            document,
                            revision,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::AcceptComposerDraft {
                    request_id,
                    expected_account,
                    target,
                    submitted_revision,
                } => {
                    if !composer_draft_account_matches(&self.state, &expected_account) {
                        return false;
                    }
                    if composer_draft_acceptance_would_exhaust(
                        &self.state,
                        &target,
                        submitted_revision,
                    ) {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::ComposerRevisionExhausted,
                            },
                        });
                        return false;
                    }
                    let effects = self
                        .reduce_app_action(AppAction::ComposerDraftAccepted {
                            target,
                            submitted_revision,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetUploadStaging {
                    request_id,
                    target,
                    items,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingChanged { target, items })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::UpdateStagedUploadCaption {
                    request_id,
                    target,
                    staged_id,
                    caption,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingCaptionChanged {
                            target,
                            staged_id,
                            caption,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::UpdateStagedUploadCompression {
                    request_id,
                    target,
                    staged_id,
                    compression_choice,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingCompressionChanged {
                            target,
                            staged_id,
                            compression_choice,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SelectStagedUploadOutput {
                    request_id,
                    target,
                    staged_id,
                    selection,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingOutputSelected {
                            target,
                            staged_id,
                            selection,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::ClearUploadStaging { request_id, target } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingCleared { target })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::ScheduleSend {
                    request_id,
                    expected_account,
                    room_id,
                    thread_root_event_id,
                    body,
                    send_at_ms,
                    draft_revision,
                } => {
                    if !composer_draft_account_matches(&self.state, &expected_account) {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        return false;
                    }
                    let target = match &thread_root_event_id {
                        Some(root_event_id) => ComposerTarget::Thread {
                            room_id: room_id.clone(),
                            root_event_id: root_event_id.clone(),
                        },
                        None => ComposerTarget::Main {
                            room_id: room_id.clone(),
                        },
                    };
                    if composer_draft_acceptance_would_exhaust(&self.state, &target, draft_revision)
                    {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::ComposerRevisionExhausted,
                            },
                        });
                        return false;
                    }
                    // Issue #450: validate slash semantics BEFORE either
                    // scheduled-send acceptance path clears the draft — a
                    // recognized-but-unavailable command (/join, /invite) is
                    // rejected terminally here instead of being scheduled and
                    // entering a permanent dispatch/retry loop. The rejection
                    // is keyed to the composer target so the UI routes the
                    // notice to the right pane (no frontend correlation
                    // needed).
                    if let Err(kind) =
                        crate::timeline::composer::validate_composer_body_for_timeline_send(&body)
                    {
                        if kind == TimelineFailureKind::UnsupportedSlashCommand
                            && let Some(key) =
                                self.composer_target_notice_key(&expected_account, &target)
                        {
                            self.emit(CoreEvent::Room(
                                koushi_protocol::event::RoomEvent::ComposerSlashCommandRejected {
                                    key,
                                    request_id,
                                },
                            ));
                        } else {
                            self.emit(CoreEvent::OperationFailed {
                                request_id,
                                failure: CoreFailure::TimelineOperationFailed { kind },
                            });
                        }
                        return false;
                    }
                    if self.state.scheduled_sends.capability
                        != ScheduledSendCapability::LocalFallback
                    {
                        let scheduled_id = scheduled_send_id();
                        let forwarded_permit = self.forward_composer_draft_permit(
                            request_id,
                            ComposerAcceptanceIdentity::ScheduledSend(scheduled_id.clone()),
                            composer_permit
                                .take()
                                .expect("server schedule command must carry its admitted permit"),
                        );
                        if !self
                            .account_actor
                            .send(AccountMessage::ScheduleServerDelayedSend {
                                request_id,
                                expected_account,
                                scheduled_id,
                                room_id,
                                thread_root_event_id,
                                body,
                                send_at_ms,
                                draft_revision,
                                composer_permit: forwarded_permit,
                            })
                            .await
                        {
                            self.emit(CoreEvent::OperationFailed {
                                request_id,
                                failure: CoreFailure::TimelineOperationFailed {
                                    kind: TimelineFailureKind::QueueOverflow,
                                },
                            });
                        }
                        return false;
                    }
                    let capability_effects = self
                        .reduce_app_action(AppAction::ScheduledSendCapabilityChanged {
                            capability: ScheduledSendCapability::LocalFallback,
                        })
                        .await;
                    self.handle_app_effects(request_id, capability_effects)
                        .await;
                    let item = ScheduledSendItem {
                        scheduled_id: scheduled_send_id(),
                        room_id,
                        thread_root_event_id,
                        body,
                        send_at_ms,
                        handle: ScheduledSendHandle::Local,
                        is_dispatching: false,
                    };
                    let effects = self
                        .reduce_app_action(AppAction::ScheduledSendCreatedAtRevision {
                            item,
                            draft_revision,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CancelScheduledSend {
                    request_id,
                    scheduled_id,
                } => {
                    if let Some(ScheduledSendHandle::Server { delay_id }) = self
                        .state
                        .scheduled_sends
                        .items
                        .get(&scheduled_id)
                        .map(|item| item.handle.clone())
                    {
                        if !self
                            .account_actor
                            .send(AccountMessage::CancelServerDelayedSend {
                                request_id,
                                scheduled_id,
                                delay_id,
                            })
                            .await
                        {
                            self.emit(CoreEvent::OperationFailed {
                                request_id,
                                failure: CoreFailure::TimelineOperationFailed {
                                    kind: TimelineFailureKind::QueueOverflow,
                                },
                            });
                        }
                        return false;
                    }
                    let effects = self
                        .reduce_app_action(AppAction::ScheduledSendCancelled { scheduled_id })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::RescheduleScheduledSend {
                    request_id,
                    scheduled_id,
                    body,
                    send_at_ms,
                } => {
                    // Issue #450: rescheduling must apply the same slash
                    // validation as the initial schedule — otherwise a
                    // recognized-but-unavailable command (/join, /invite)
                    // could be stored and enter the permanent dispatch/retry
                    // loop. Reject terminally and leave the existing item
                    // untouched. Scheduled items are edited from the main-pane
                    // scheduled list (thread items included), so the notice is
                    // keyed to the item's room — visible without the thread
                    // being open.
                    if let Err(kind) =
                        crate::timeline::composer::validate_composer_body_for_timeline_send(&body)
                    {
                        let notice_key =
                            composer_draft_session_key(&self.state).and_then(|account| {
                                self.state
                                    .scheduled_sends
                                    .items
                                    .get(&scheduled_id)
                                    .and_then(|item| {
                                        self.composer_target_notice_key(
                                            &account,
                                            &ComposerTarget::Main {
                                                room_id: item.room_id.clone(),
                                            },
                                        )
                                    })
                            });
                        if kind == TimelineFailureKind::UnsupportedSlashCommand
                            && let Some(key) = notice_key
                        {
                            self.emit(CoreEvent::Room(
                                koushi_protocol::event::RoomEvent::ComposerSlashCommandRejected {
                                    key,
                                    request_id,
                                },
                            ));
                        } else {
                            self.emit(CoreEvent::OperationFailed {
                                request_id,
                                failure: CoreFailure::TimelineOperationFailed { kind },
                            });
                        }
                        return false;
                    }
                    if let Some(item) = self.state.scheduled_sends.items.get(&scheduled_id).cloned()
                        && let ScheduledSendHandle::Server { delay_id } = item.handle
                    {
                        if !self
                            .account_actor
                            .send(AccountMessage::RescheduleServerDelayedSend {
                                request_id,
                                scheduled_id,
                                room_id: item.room_id,
                                thread_root_event_id: item.thread_root_event_id,
                                body,
                                delay_id,
                                send_at_ms,
                            })
                            .await
                        {
                            self.emit(CoreEvent::OperationFailed {
                                request_id,
                                failure: CoreFailure::TimelineOperationFailed {
                                    kind: TimelineFailureKind::QueueOverflow,
                                },
                            });
                        }
                        return false;
                    }
                    let effects = self
                        .reduce_app_action(AppAction::ScheduledSendRescheduled {
                            scheduled_id,
                            body,
                            send_at_ms,
                            handle: ScheduledSendHandle::Local,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::OpenThread {
                    request_id,
                    room_id,
                    root_event_id,
                    intent,
                } => {
                    let replaced_thread_key =
                        self.unsubscribe_replaced_thread_timeline(&room_id, &root_event_id);
                    let effects = self
                        .reduce_app_action(AppAction::OpenThread {
                            room_id,
                            root_event_id,
                            intent,
                        })
                        .await;
                    if effects_open_thread_timeline(&effects) {
                        if let Some(key) = replaced_thread_key {
                            self.send_timeline_command_or_fail(
                                request_id,
                                TimelineCommand::Unsubscribe { request_id, key },
                            )
                            .await;
                        }
                    }
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CloseThread { request_id } => {
                    let thread_key = self.current_thread_timeline_key();
                    let effects = self.reduce_app_action(AppAction::CloseThread).await;
                    if let Some(key) = thread_key {
                        self.send_timeline_command_or_fail(
                            request_id,
                            TimelineCommand::Unsubscribe { request_id, key },
                        )
                        .await;
                    }
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::OpenFocusedContext {
                    request_id,
                    room_id,
                    event_id,
                } => {
                    self.pending_focused_navigation = None;
                    if !self
                        .ensure_room_event_cached(request_id, &room_id, &event_id)
                        .await
                    {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::Timeout,
                            },
                        });
                        return true;
                    }
                    let replaced_focused_key =
                        self.unsubscribe_replaced_focused_context_timeline(&room_id, &event_id);
                    let effects = self
                        .reduce_app_action(AppAction::OpenFocusedContext { room_id, event_id })
                        .await;
                    if effects_open_focused_timeline(&effects) {
                        if let Some(key) = replaced_focused_key {
                            self.send_timeline_command_or_fail(
                                request_id,
                                TimelineCommand::Unsubscribe { request_id, key },
                            )
                            .await;
                        }
                    } else {
                        self.pending_focused_navigation = None;
                    }
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::OpenAnchoredTimeline {
                    request_id,
                    room_id,
                    event_id,
                    allow_live_fallback,
                } => {
                    if !self
                        .ensure_room_event_cached(request_id, &room_id, &event_id)
                        .await
                    {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::Timeout,
                            },
                        });
                        return true;
                    }
                    let replaced_focused_key =
                        self.unsubscribe_replaced_focused_context_timeline(&room_id, &event_id);
                    let Some(account_key) = self.current_account_key() else {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        return true;
                    };
                    let key = TimelineKey {
                        account_key,
                        kind: TimelineKind::Focused {
                            room_id: room_id.clone(),
                            event_id: event_id.clone(),
                        },
                    };
                    self.pending_focused_navigation = Some(PendingFocusedNavigation {
                        projection_request_id: request_id,
                        key,
                        room_id: room_id.clone(),
                        event_id: event_id.clone(),
                        allow_live_fallback,
                        generation: None,
                    });
                    let effects = self
                        .reduce_app_action(AppAction::OpenFocusedContext { room_id, event_id })
                        .await;
                    if effects_open_focused_timeline(&effects) {
                        if let Some(key) = replaced_focused_key {
                            self.send_timeline_command_or_fail(
                                request_id,
                                TimelineCommand::Unsubscribe { request_id, key },
                            )
                            .await;
                        }
                    } else {
                        self.pending_focused_navigation = None;
                    }
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::EnterAnchoredTimeline {
                    request_id,
                    room_id,
                    event_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::EnterAnchoredTimeline { room_id, event_id })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::RepairRoomTimeline {
                    request_id,
                    room_id,
                } => {
                    let Some(account_key) = self.current_account_key() else {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        return true;
                    };
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RepairRoomTimeline {
                            request_id,
                            account_key,
                            room_id,
                        })
                        .await;
                    true
                }
                AppCommand::OpenTimelineAtTimestamp {
                    request_id,
                    room_id,
                    timestamp_ms,
                } => {
                    let focused_key = self.current_focused_context_timeline_key();
                    let effects = self.reduce_app_action(AppAction::CloseFocusedContext).await;
                    if let Some(key) = focused_key {
                        self.send_timeline_command_or_fail(
                            request_id,
                            TimelineCommand::Unsubscribe { request_id, key },
                        )
                        .await;
                    }
                    self.handle_app_effects(request_id, effects).await;
                    if let Some(event_id) = self
                        .activity_projection
                        .event_at_or_after(&room_id, timestamp_ms)
                    {
                        // #161: jump-to-date reuses the focused-context timeline
                        // subscription lifecycle but renders it in the MAIN pane
                        // (marked by `main_timeline_anchor`), not the right panel.
                        let Some(account_key) = self.current_account_key() else {
                            self.emit(CoreEvent::OperationFailed {
                                request_id,
                                failure: CoreFailure::SessionRequired,
                            });
                            return true;
                        };
                        self.pending_focused_navigation = Some(PendingFocusedNavigation {
                            projection_request_id: request_id,
                            key: TimelineKey {
                                account_key,
                                kind: TimelineKind::Focused {
                                    room_id: room_id.clone(),
                                    event_id: event_id.clone(),
                                },
                            },
                            room_id: room_id.clone(),
                            event_id: event_id.clone(),
                            allow_live_fallback: true,
                            generation: None,
                        });
                        let effects = self
                            .reduce_app_action(AppAction::OpenFocusedContext {
                                room_id: room_id.clone(),
                                event_id: event_id.clone(),
                            })
                            .await;
                        self.handle_app_effects(request_id, effects).await;
                        return true;
                    }
                    self.pending_date_navigation_request_id = Some(request_id);
                    let _ = self
                        .account_actor
                        .send(AccountMessage::OpenTimelineAtTimestamp {
                            request_id,
                            room_id,
                            timestamp_ms,
                        })
                        .await;
                    true
                }
                AppCommand::TimelineScrollAnchorUpdated {
                    request_id,
                    room_id,
                    anchor,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::TimelineScrollAnchorUpdated {
                            room_id,
                            anchor,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CloseFocusedContext { request_id } => {
                    self.pending_focused_navigation = None;
                    let focused_key = self.current_focused_context_timeline_key();
                    let effects = self.reduce_app_action(AppAction::CloseFocusedContext).await;
                    if let Some(key) = focused_key {
                        self.send_timeline_command_or_fail(
                            request_id,
                            TimelineCommand::Unsubscribe { request_id, key },
                        )
                        .await;
                    }
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CloseSearch { request_id } => {
                    let effects = self.reduce_app_action(AppAction::SearchClosed).await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::OpenInviteWorkflow {
                    request_id,
                    room_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::InviteWorkflowOpened { room_id })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CloseInviteWorkflow { request_id } => {
                    let effects = self
                        .reduce_app_action(AppAction::InviteWorkflowClosed)
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SearchInviteTargets {
                    request_id,
                    room_id,
                    query,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::InviteTargetQueryChanged { room_id, query })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetInviteScope {
                    request_id,
                    room_id,
                    scope,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::InviteScopeSelected { room_id, scope })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SelectInviteTarget {
                    request_id,
                    room_id,
                    user_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::InviteTargetSelected { room_id, user_id })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::RemoveInviteTarget {
                    request_id,
                    user_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::InviteTargetRemoved { user_id })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::UpdateSettings { request_id, patch } => {
                    let effects = self
                        .reduce_app_action(AppAction::SettingsUpdateRequested {
                            request_id: request_id.sequence,
                            patch,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::ImportLegacySettings { request_id, patch } => {
                    if self.settings_load_status == SettingsLoadStatus::Failed {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::StoreUnavailable,
                        });
                        true
                    } else if self
                        .state
                        .settings
                        .values
                        .legacy_frontend_preferences_imported
                    {
                        true
                    } else {
                        let mut values = self.state.settings.values.clone();
                        values.apply_patch(patch);
                        values.legacy_frontend_preferences_imported = true;
                        let projected_values = values.clone();
                        let store = self.settings_store.clone();
                        let saved = executor::spawn_blocking(move || store.save(&values)).await;
                        match saved {
                            Ok(Ok(())) => {
                                let effects = self
                                    .reduce_app_action(AppAction::SettingsLoaded {
                                        values: projected_values,
                                    })
                                    .await;
                                self.handle_app_effects(request_id, effects).await;
                            }
                            Ok(Err(_)) | Err(_) => {
                                self.emit(CoreEvent::OperationFailed {
                                    request_id,
                                    failure: CoreFailure::StoreUnavailable,
                                });
                            }
                        }
                        true
                    }
                }
                AppCommand::UpdateNavigationPreference { request_id, update } => {
                    self.handle_navigation_preference_command(request_id, update)
                        .await;
                    true
                }
                AppCommand::RebuildSearchIndex { request_id } => {
                    let effects = self
                        .reduce_app_action(AppAction::SearchIndexRebuildRequested {
                            request_id: request_id.sequence,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetRoomUrlPreviewOverride {
                    request_id,
                    room_id,
                    enabled,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::RoomUrlPreviewOverrideSet {
                            request_id: request_id.sequence,
                            room_id,
                            enabled,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::OpenActivity { request_id } => {
                    let previous_tab = match self.state.activity {
                        ActivityState::Closed { last_selected_tab } => last_selected_tab,
                        ActivityState::Opening { tab, .. } => {
                            record_activity_transition(
                                "open_applied",
                                request_id,
                                "already_opening",
                                tab,
                                tab,
                            );
                            return true;
                        }
                        ActivityState::Open { active_tab, .. } => {
                            record_activity_transition(
                                "open_applied",
                                request_id,
                                "already_open",
                                active_tab,
                                active_tab,
                            );
                            return true;
                        }
                    };
                    let effects = self
                        .reduce_app_action(AppAction::ActivityOpened {
                            request_id: request_id.sequence,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    let opening_tab = match self.state.activity {
                        ActivityState::Opening {
                            request_id: active_request_id,
                            tab,
                        } if active_request_id == request_id.sequence => tab,
                        _ => {
                            record_activity_transition(
                                "open_applied",
                                request_id,
                                "stale",
                                previous_tab,
                                previous_tab,
                            );
                            return true;
                        }
                    };
                    let (recent, unread, excluded_room_ids) =
                        self.activity_projection.snapshot(&self.state);
                    let snapshot_effects = self
                        .reduce_app_action(AppAction::ActivitySnapshotLoaded {
                            request_id: request_id.sequence,
                            active_tab: opening_tab,
                            recent: recent.clone(),
                            unread: unread.clone(),
                            excluded_room_ids,
                        })
                        .await;
                    self.handle_app_effects(request_id, snapshot_effects).await;
                    self.start_activity_resolution().await;
                    self.emit(CoreEvent::Activity(ActivityEvent::Opened { request_id }));
                    let (recent, unread, selected_tab) = match &self.state.activity {
                        ActivityState::Open {
                            active_tab,
                            recent,
                            unread,
                            ..
                        } => (recent.clone(), unread.clone(), *active_tab),
                        _ => (recent, unread, opening_tab),
                    };
                    record_activity_transition(
                        "open_applied",
                        request_id,
                        "opened",
                        previous_tab,
                        selected_tab,
                    );
                    self.emit(CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                        request_id,
                        active_tab: selected_tab,
                        recent,
                        unread,
                    }));
                    true
                }
                AppCommand::CloseActivity { request_id } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::CancelActivityResolution)
                        .await;
                    let effects = self.reduce_app_action(AppAction::ActivityClosed).await;
                    self.handle_app_effects(request_id, effects).await;
                    self.emit(CoreEvent::Activity(ActivityEvent::Closed { request_id }));
                    true
                }
                AppCommand::SetActivityTab { request_id, tab } => {
                    let previous_tab = match self.state.activity {
                        ActivityState::Open { active_tab, .. } => Some(active_tab),
                        _ => None,
                    };
                    let effects = self
                        .reduce_app_action(AppAction::ActivityTabSelected { tab })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    if let Some(previous_tab) = previous_tab {
                        if previous_tab != tab {
                            record(
                                DiagnosticEvent::new(
                                    DiagnosticLevel::Info,
                                    "core.activity",
                                    "tab_selected",
                                )
                                .field(DiagnosticField::request_id(
                                    "request_id",
                                    request_id.connection_id.0,
                                    request_id.sequence,
                                ))
                                .field(DiagnosticField::token(
                                    "previous_tab",
                                    activity_tab_token(previous_tab),
                                ))
                                .field(DiagnosticField::token(
                                    "selected_tab",
                                    activity_tab_token(tab),
                                )),
                            );
                        }
                    }
                    self.emit(CoreEvent::Activity(ActivityEvent::TabSelected {
                        request_id,
                        tab,
                    }));
                    true
                }
                AppCommand::PaginateActivity {
                    request_id, tab, ..
                } => {
                    let (recent, unread, excluded_room_ids) =
                        self.activity_projection.snapshot(&self.state);
                    let effects = self
                        .reduce_app_action(AppAction::ActivityRowsUpdated {
                            recent: recent.clone(),
                            unread: unread.clone(),
                            excluded_room_ids,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    self.emit(CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                        request_id,
                        active_tab: tab,
                        recent,
                        unread,
                    }));
                    true
                }
                AppCommand::RetryActivityResolution { request_id } => {
                    self.start_activity_resolution().await;
                    self.emit(CoreEvent::Activity(ActivityEvent::ResolutionRetried {
                        request_id,
                        generation: self.activity_resolution_generation,
                    }));
                    true
                }
                AppCommand::MarkActivityRead { request_id, target } => {
                    let effects = self
                        .reduce_app_action(AppAction::ActivityMarkReadRequested {
                            request_id: request_id.sequence,
                            target: target.clone(),
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    let fully_read_updates = self
                        .activity_projection
                        .fully_read_marker_updates(&self.state, &target);
                    let mark_read_result = self.activity_projection.mark_read(&self.state, &target);
                    let cleared_room_ids =
                        self.activity_projection.room_ids_without_remaining_unread(
                            &self.state,
                            &mark_read_result.cleared_event_ids,
                        );
                    let success_effects = self
                        .reduce_app_action(AppAction::ActivityMarkReadSucceeded {
                            request_id: request_id.sequence,
                            cleared_event_ids: mark_read_result.cleared_event_ids.clone(),
                        })
                        .await;
                    self.handle_app_effects(request_id, success_effects).await;
                    for room_id in mark_read_result.cleared_placeholder_room_ids {
                        let room_effects = self
                            .reduce_app_action(AppAction::RoomMarkedAsReadSucceeded {
                                request_id: request_id.sequence,
                                room_id,
                            })
                            .await;
                        self.handle_app_effects(request_id, room_effects).await;
                    }
                    for room_id in cleared_room_ids {
                        let room_effects = self
                            .reduce_app_action(AppAction::RoomMarkedAsReadSucceeded {
                                request_id: request_id.sequence,
                                room_id,
                            })
                            .await;
                        self.handle_app_effects(request_id, room_effects).await;
                    }
                    for (room_id, event_id) in fully_read_updates {
                        let room_read_request_id = self.next_internal_request_id();
                        let _ = self
                            .account_actor
                            .send(AccountMessage::RoomCommand(
                                koushi_protocol::command::RoomCommand::MarkRoomAsRead {
                                    request_id: room_read_request_id,
                                    room_id: room_id.clone(),
                                    event_id: event_id.clone(),
                                },
                            ))
                            .await;
                        let marker_effects = self
                            .reduce_app_action(AppAction::FullyReadMarkerUpdated {
                                room_id,
                                event_id: Some(event_id),
                            })
                            .await;
                        self.handle_app_effects(request_id, marker_effects).await;
                    }
                    if let Some(activity_update) = self
                        .activity_projection
                        .update_action_for_open_state(&self.state)
                    {
                        let activity_update_effects = self.reduce_app_action(activity_update).await;
                        self.handle_app_effects(request_id, activity_update_effects)
                            .await;
                    }
                    self.emit(CoreEvent::Activity(ActivityEvent::MarkedRead {
                        request_id,
                        cleared_event_ids: mark_read_result.cleared_event_ids,
                    }));
                    true
                }
                AppCommand::OpenFilesView {
                    request_id,
                    scope,
                    filter,
                    sort,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::FilesViewOpened {
                            request_id: request_id.sequence,
                            scope,
                            filter,
                            sort,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CloseFilesView { request_id } => {
                    let effects = self.reduce_app_action(AppAction::FilesViewClosed).await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::OpenThreadsList { request_id, scope } => {
                    let effects = self
                        .reduce_app_action(AppAction::OpenThreadsList {
                            request_id: request_id.sequence,
                            room_id: scope.scope_key(),
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CloseThreadsList { request_id } => {
                    let effects = self.reduce_app_action(AppAction::CloseThreadsList).await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::PaginateThreadsList { request_id, scope } => {
                    let effects = self
                        .reduce_app_action(AppAction::PaginateThreadsList {
                            request_id: request_id.sequence,
                            room_id: scope.scope_key(),
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::RecordLocalEncryptionHealth { request_id, health } => {
                    let probe_effects = self
                        .reduce_app_action(AppAction::LocalEncryptionProbeRequested {
                            request_id: request_id.sequence,
                        })
                        .await;
                    self.handle_app_effects(request_id, probe_effects).await;
                    let health_effects = self
                        .reduce_app_action(AppAction::LocalEncryptionHealthChanged {
                            request_id: request_id.sequence,
                            health,
                        })
                        .await;
                    self.handle_app_effects(request_id, health_effects).await;
                    true
                }
                AppCommand::UpdateNativeAttentionState {
                    request_id,
                    attention,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::NativeAttentionUpdated { attention })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::ObserveNativeWindowFocus {
                    request_id,
                    focused,
                    observation_generation,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::NativeWindowFocusChanged {
                            focused,
                            observation_generation,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::StartNativeAttentionDispatch {
                    request_id,
                    dispatch_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::NativeAttentionDispatchStarted {
                            dispatch_id,
                        })
                        .await;
                    self.emit(CoreEvent::NativeAttention(
                        NativeAttentionEvent::DispatchAdmission {
                            dispatch_id,
                            accepted: !effects.is_empty(),
                        },
                    ));
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SettleNativeAttentionDispatch {
                    request_id,
                    dispatch_id,
                    outcome,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::NativeAttentionDispatchSettled {
                            dispatch_id,
                            outcome,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::UpdateJapaneseCatalogProfile {
                    request_id,
                    profile,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::JapaneseCatalogProfileChanged { profile })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SelectRoomListFilter { request_id, filter } => {
                    let effects = self
                        .reduce_app_action(AppAction::RoomListFilterSelected { filter })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
            },
            CoreCommand::Sync(sync_command) => {
                // Route to AccountActor (which forwards to SyncActor).
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::SyncCommand(sync_command))
                    .await;
                false
            }
            CoreCommand::Room(room_command) => {
                let mut state_changed = false;
                match &room_command {
                    koushi_protocol::command::RoomCommand::LoadSpaceMembers {
                        request_id,
                        space_id,
                        generation,
                    } => {
                        if let Err(rejection) =
                            admit_space_members_load(&self.state, space_id, *generation)
                        {
                            record_space_member_command_rejection("load", rejection);
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        let effects = self
                            .reduce_app_action(AppAction::SpaceMembersLoadRequested {
                                request_id: request_id.sequence,
                                space_id: space_id.clone(),
                                generation: *generation,
                            })
                            .await;
                        if effects.is_empty() {
                            record_space_member_command_rejection(
                                "load",
                                SpaceMembersCommandRejection::StaleGeneration,
                            );
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        self.handle_ui_event_effects(&effects).await;
                        state_changed = true;
                    }
                    koushi_protocol::command::RoomCommand::InviteUserToSpace {
                        request_id,
                        space_id,
                        user_id,
                        generation,
                    } => {
                        if let Err(rejection) = admit_space_member_invite(
                            &self.state.space_members,
                            space_id,
                            user_id,
                            *generation,
                        ) {
                            record_space_member_command_rejection("invite", rejection);
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        let effects = self
                            .reduce_app_action(AppAction::SpaceMemberInviteRequested {
                                request_id: request_id.sequence,
                                space_id: space_id.clone(),
                                user_id: user_id.clone(),
                                generation: *generation,
                            })
                            .await;
                        if effects.is_empty() {
                            record_space_member_command_rejection(
                                "invite",
                                SpaceMembersCommandRejection::InviteAlreadyInFlight,
                            );
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        self.handle_ui_event_effects(&effects).await;
                        state_changed = true;
                    }
                    koushi_protocol::command::RoomCommand::CancelSpaceInvite {
                        request_id,
                        space_id,
                        user_id,
                        generation,
                    } => {
                        if let Err(rejection) = admit_space_member_cancellation(
                            &self.state.space_members,
                            space_id,
                            user_id,
                            *generation,
                        ) {
                            record_space_member_command_rejection("cancel", rejection);
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        let effects = self
                            .reduce_app_action(AppAction::SpaceMemberInviteCancellationRequested {
                                request_id: request_id.sequence,
                                space_id: space_id.clone(),
                                user_id: user_id.clone(),
                                generation: *generation,
                            })
                            .await;
                        if effects.is_empty() {
                            record_space_member_command_rejection(
                                "cancel",
                                SpaceMembersCommandRejection::CancellationAlreadyInFlight,
                            );
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        self.handle_ui_event_effects(&effects).await;
                        state_changed = true;
                    }
                    koushi_protocol::command::RoomCommand::UpdateSpaceMemberRole {
                        request_id,
                        space_id,
                        user_id,
                        generation,
                        expected_power_levels_revision,
                        expected_power_level,
                        power_level,
                        confirmed,
                    } => {
                        if let Err(rejection) = admit_space_member_role(
                            &self.state,
                            space_id,
                            user_id,
                            *generation,
                            expected_power_levels_revision.as_deref(),
                            *expected_power_level,
                            *power_level,
                            *confirmed,
                        ) {
                            record_space_member_command_rejection("role_update", rejection);
                            let failure =
                                if rejection == SpaceMembersCommandRejection::RoleSessionRequired {
                                    CoreFailure::SessionRequired
                                } else {
                                    CoreFailure::RoomOperationFailed {
                                        kind: match rejection {
                                            SpaceMembersCommandRejection::RoleNotEditable => {
                                                RoomFailureKind::Forbidden
                                            }
                                            SpaceMembersCommandRejection::RoleTargetInvalid => {
                                                RoomFailureKind::NotFound
                                            }
                                            _ => RoomFailureKind::Sdk,
                                        },
                                    }
                                };
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure,
                            });
                            return false;
                        }
                        let effects = self
                            .reduce_app_action(AppAction::SpaceMemberRoleUpdateRequested {
                                request_id: request_id.sequence,
                                space_id: space_id.clone(),
                                user_id: user_id.clone(),
                                generation: *generation,
                                expected_power_levels_revision: expected_power_levels_revision
                                    .clone(),
                                expected_power_level: *expected_power_level,
                                power_level: *power_level,
                                confirmed: *confirmed,
                            })
                            .await;
                        if effects.is_empty() {
                            record_space_member_command_rejection(
                                "role_update",
                                SpaceMembersCommandRejection::RoleUpdateAlreadyInFlight,
                            );
                            self.emit(CoreEvent::OperationFailed {
                                request_id: *request_id,
                                failure: CoreFailure::RoomOperationFailed {
                                    kind: RoomFailureKind::Sdk,
                                },
                            });
                            return false;
                        }
                        self.handle_ui_event_effects(&effects).await;
                        state_changed = true;
                    }
                    _ => {}
                }
                // User-intent lane: for SelectRoom, record the request_id→room_id
                // correlation BEFORE forwarding so the action loop can emit the
                // terminal IntentLifecycle outcome. This command path is reliable
                // and must never be converted into a drop-on-full background path.
                if let koushi_protocol::command::RoomCommand::SelectRoom {
                    request_id,
                    ref room_id,
                } = room_command
                {
                    self.pending_select
                        .entry(room_id.clone())
                        .or_default()
                        .push_back(request_id);
                }
                let forward_failure = space_member_forward_failure_action(&room_command);
                // Route to AccountActor (which forwards to RoomActor).
                let forwarded = self
                    .account_actor
                    .send(crate::account::AccountMessage::RoomCommand(room_command))
                    .await;
                if !forwarded {
                    if let Some((request_id, failure_action)) = forward_failure {
                        let effects = self.reduce_app_action(failure_action).await;
                        self.handle_ui_event_effects(&effects).await;
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::RoomOperationFailed {
                                kind: RoomFailureKind::Sdk,
                            },
                        });
                        state_changed = true;
                    }
                }
                state_changed
            }
            CoreCommand::Timeline(timeline_command) => {
                if let Some((request_id, expected_account)) =
                    timeline_composer_account_fence(&timeline_command)
                    && !composer_draft_account_matches(&self.state, expected_account)
                {
                    self.emit(CoreEvent::OperationFailed {
                        request_id,
                        failure: CoreFailure::SessionRequired,
                    });
                    return false;
                }
                if let Some((request_id, key, submission_id)) =
                    timeline_submission_revision_exhaustion(&self.state, &timeline_command)
                {
                    self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                        request_id,
                        key,
                        submission_id,
                        kind: TimelineFailureKind::ComposerRevisionExhausted,
                    }));
                    return false;
                }
                if self.should_suppress_timeline_command_for_privacy(&timeline_command) {
                    return false;
                }
                let formatting_options = self.state.settings.values.composer.formatting_options();
                // Route to AccountActor (which forwards to TimelineManagerActor).
                let message = if let Some(permit) = composer_permit.take() {
                    let request_id = timeline_composer_account_fence(&timeline_command)
                        .map(|(request_id, _)| request_id)
                        .expect("leased timeline command must have an account fence");
                    let identity =
                        composer_acceptance_identity_for_timeline_command(&timeline_command)
                            .expect("leased timeline command must have an acceptance identity");
                    crate::account::AccountMessage::LeasedTimelineCommandWithComposerFormatting {
                        command: timeline_command,
                        composer_permit: self
                            .forward_composer_draft_permit(request_id, identity, permit),
                        formatting_options,
                    }
                } else {
                    crate::account::AccountMessage::TimelineCommandWithComposerFormatting {
                        command: timeline_command,
                        formatting_options,
                    }
                };
                let _ = self.account_actor.send(message).await;
                false
            }
            CoreCommand::Search(search_command) => {
                match search_command {
                    SearchCommand::Query {
                        request_id,
                        query,
                        scope,
                        ..
                    } => {
                        let effects = self
                            .reduce_app_action(AppAction::SearchSubmitted {
                                request_id: request_id.sequence,
                                query: query.clone(),
                                scope: search_scope_to_state(&scope),
                            })
                            .await;
                        self.handle_app_effects(request_id, effects).await;
                        true
                    }
                    SearchCommand::Attachments { .. } => {
                        // Attachments are driven by `AppAction::FilesViewOpened` in
                        // Phase A; a direct `CoreCommand::Search(Attachments)` is not
                        // wired to the reducer.
                        false
                    }
                    SearchCommand::StartHistoryCrawl { .. }
                    | SearchCommand::StopHistoryCrawl { .. } => {
                        // Forward directly to the SearchActor; the crawler task sends
                        // HistoryCrawlStarted/Progress/Completed/Failed actions itself.
                        let _ = self
                            .account_actor
                            .send(crate::account::AccountMessage::SearchCommand(
                                search_command,
                            ))
                            .await;
                        false
                    }
                }
            }
        }
    }

    fn should_suppress_timeline_command_for_privacy(
        &self,
        command: &koushi_protocol::command::TimelineCommand,
    ) -> bool {
        match command {
            koushi_protocol::command::TimelineCommand::SendReadReceipt { .. } => {
                !self.state.settings.values.notifications.send_read_receipts
            }
            koushi_protocol::command::TimelineCommand::SetTyping { .. } => {
                !self
                    .state
                    .settings
                    .values
                    .notifications
                    .send_typing_notifications
            }
            _ => false,
        }
    }

    async fn handle_app_effects(&mut self, request_id: RequestId, effects: Vec<AppEffect>) {
        for effect in effects {
            match effect {
                AppEffect::ContinueSlidingSyncAdmission {
                    account_epoch,
                    request_id,
                    source,
                    ..
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::ContinueSlidingSyncAdmission {
                            account_epoch,
                            request_id,
                            source,
                        })
                        .await;
                }
                AppEffect::RetrySlidingSyncCapabilityDiscovery {
                    account_epoch,
                    blocked_request_id,
                    request_id,
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RetrySlidingSyncCapabilityDiscovery {
                            account_epoch,
                            blocked_request_id,
                            request_id,
                        })
                        .await;
                }
                AppEffect::ScheduleSlidingSyncCapabilityRevalidation { account_epoch } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::ScheduleSlidingSyncCapabilityRevalidation {
                            account_epoch,
                        })
                        .await;
                }
                AppEffect::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SettleSlidingSyncCapabilityRevalidation {
                            account_epoch,
                            request_id,
                            result,
                        })
                        .await;
                }
                AppEffect::StartSync => {
                    trace_runtime_sync!(
                        "effect_start_sync",
                        [
                            DiagnosticField::token("source", "command_effect"),
                            DiagnosticField::request_id(
                                "request_id",
                                request_id.connection_id.0,
                                request_id.sequence
                            ),
                            DiagnosticField::token("action", "send_sync_start"),
                        ],
                        "source=command_effect request_id={} action=send_sync_start",
                        runtime_request_id_trace_label(request_id)
                    );
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SyncCommand(SyncCommand::Start {
                            request_id,
                        }))
                        .await;
                }
                AppEffect::StopSync => {
                    trace_runtime_sync!(
                        "effect_stop_sync",
                        [
                            DiagnosticField::token("source", "command_effect"),
                            DiagnosticField::request_id(
                                "request_id",
                                request_id.connection_id.0,
                                request_id.sequence
                            ),
                            DiagnosticField::token("action", "send_sync_stop"),
                        ],
                        "source=command_effect request_id={} action=send_sync_stop",
                        runtime_request_id_trace_label(request_id)
                    );
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SyncCommand(SyncCommand::Stop {
                            request_id,
                        }))
                        .await;
                }
                AppEffect::SubscribeTimeline { room_id } => {
                    let Some(account_key) = self.current_account_key() else {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        continue;
                    };
                    self.send_timeline_command_or_fail(
                        request_id,
                        TimelineCommand::EnsureSubscribed {
                            request_id,
                            key: TimelineKey {
                                account_key,
                                kind: TimelineKind::Room { room_id },
                            },
                            replay_existing: false,
                        },
                    )
                    .await;
                }
                AppEffect::OpenThreadTimeline {
                    room_id,
                    root_event_id,
                    intent,
                } => {
                    let Some(account_key) = self.current_account_key() else {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        continue;
                    };
                    self.send_timeline_command_or_fail(
                        request_id,
                        TimelineCommand::Subscribe {
                            request_id,
                            key: TimelineKey {
                                account_key,
                                kind: TimelineKind::Thread {
                                    room_id,
                                    root_event_id,
                                },
                            },
                            initial_backfill: match intent {
                                ThreadOpenIntent::ExistingThread
                                | ThreadOpenIntent::PinnedReply { .. } => {
                                    koushi_protocol::command::InitialBackfillPolicy::RequiredForExistingThread
                                }
                                ThreadOpenIntent::NewThreadDraft => {
                                    koushi_protocol::command::InitialBackfillPolicy::Disabled
                                }
                            },
                        },
                    )
                    .await;
                }
                AppEffect::OpenFocusedTimeline { room_id, event_id } => {
                    let Some(account_key) = self.current_account_key() else {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        continue;
                    };
                    self.send_timeline_command_or_fail(
                        request_id,
                        TimelineCommand::Subscribe {
                            request_id,
                            key: TimelineKey {
                                account_key,
                                kind: TimelineKind::Focused { room_id, event_id },
                            },
                            initial_backfill:
                                koushi_protocol::command::InitialBackfillPolicy::Disabled,
                        },
                    )
                    .await;
                }
                AppEffect::SearchMessages {
                    request_id: effect_request_id,
                    query,
                    scope,
                    room_filter,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::SearchCommand(
                            SearchCommand::Query {
                                request_id,
                                query,
                                scope: map_state_search_scope_to_core(scope),
                                room_filter,
                            },
                        ))
                        .await;
                }
                AppEffect::SearchAttachments {
                    request_id: effect_request_id,
                    scope,
                    filter,
                    sort,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::SearchCommand(
                            SearchCommand::Attachments {
                                request_id,
                                scope,
                                filter,
                                sort,
                            },
                        ))
                        .await;
                }
                AppEffect::SubscribeThreadsList {
                    request_id: effect_request_id,
                    room_id,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::ThreadsListCommand(
                            koushi_protocol::command::ThreadsListCommand::Open {
                                request_id,
                                scope: koushi_state::ThreadsListScope::Room {
                                    room_id: room_id.clone(),
                                },
                                room_ids: vec![room_id],
                            },
                        ))
                        .await;
                }
                AppEffect::SubscribeThreadsListScoped {
                    request_id: effect_request_id,
                    scope,
                    room_ids,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::ThreadsListCommand(
                            koushi_protocol::command::ThreadsListCommand::Open {
                                request_id,
                                scope,
                                room_ids,
                            },
                        ))
                        .await;
                }
                AppEffect::PaginateThreadsList {
                    request_id: effect_request_id,
                    room_id,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::ThreadsListCommand(
                            koushi_protocol::command::ThreadsListCommand::Paginate {
                                request_id,
                                scope: koushi_state::ThreadsListScope::from_scope_key(&room_id),
                            },
                        ))
                        .await;
                }
                AppEffect::UnsubscribeThreadsList => {
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::ThreadsListCommand(
                            koushi_protocol::command::ThreadsListCommand::Close { request_id },
                        ))
                        .await;
                }
                AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, settings } => {
                    let _ = self
                        .account_actor
                        .send(
                            crate::account::AccountMessage::NotifySearchCrawlerRoomsAvailable {
                                room_ids,
                                settings,
                            },
                        )
                        .await;
                }
                AppEffect::InvalidateSearchCrawlerCache => {
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::InvalidateSearchCrawlerCache)
                        .await;
                }
                AppEffect::RebuildSearchIndex => {
                    let _ = self
                        .account_actor
                        .send(crate::account::AccountMessage::RebuildSearchIndex)
                        .await;
                }
                AppEffect::PersistSettings {
                    request_id: effect_request_id,
                    values,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    let settings_store = self.settings_store.clone();
                    let action = match executor::spawn_blocking(move || {
                        settings_store.save(&values)
                    })
                    .await
                    {
                        Ok(Ok(())) => AppAction::SettingsPersisted {
                            request_id: effect_request_id,
                        },
                        Ok(Err(_)) | Err(_) => AppAction::SettingsPersistFailed {
                            request_id: effect_request_id,
                            message: "settings could not be saved".to_owned(),
                        },
                    };
                    let _ = self.reduce_app_action(action).await;
                }
                AppEffect::PersistRoomPreferences {
                    request_id: effect_request_id,
                    preferences,
                } => {
                    if effect_request_id != request_id.sequence {
                        continue;
                    }
                    self.persist_room_preferences(&preferences).await;
                }
                AppEffect::EmitUiEvent(ui_event) => {
                    self.handle_ui_event_effect(&ui_event, &[]).await;
                }
                AppEffect::RejectProvisionalSession => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RejectProvisionalSession { request_id })
                        .await;
                }
                AppEffect::CheckCurrentDeviceTrust => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::CheckCurrentDeviceTrust)
                        .await;
                }
                AppEffect::InspectSecureBackup => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::InspectSecureBackup)
                        .await;
                }
                AppEffect::SyncConnectivityChanged { proven } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SyncConnectivityChanged { proven })
                        .await;
                }
                AppEffect::RefreshCurrentSessionStatus {
                    request_id,
                    trigger,
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RefreshCurrentSessionStatus {
                            request_id,
                            trigger,
                            sync_state: current_session_sync_state(&self.state.sync),
                        })
                        .await;
                }
                AppEffect::RestoreSession
                | AppEffect::DiscoverLogin { .. }
                | AppEffect::Login { .. }
                | AppEffect::DiscoverVerificationMethods
                | AppEffect::BeginSessionVerification { .. }
                | AppEffect::RecoverE2ee(_)
                | AppEffect::RequestVerification { .. }
                | AppEffect::AcceptVerification { .. }
                | AppEffect::ConfirmSasVerification { .. }
                | AppEffect::CancelVerification { .. }
                | AppEffect::BootstrapCrossSigning { .. }
                | AppEffect::EnableKeyBackup { .. }
                | AppEffect::RestoreKeyBackup { .. }
                | AppEffect::ResetIdentity { .. }
                | AppEffect::PersistSession(_)
                | AppEffect::PaginateTimelineBackwards { .. }
                | AppEffect::SendText { .. }
                | AppEffect::RecordNativeAttentionRecomputed { .. } => {}
            }
        }
    }

    async fn handle_post_projection_effects(
        &mut self,
        effects: &[AppEffect],
        navigation_projection_generation: Option<u64>,
        navigation_projection_cause: Option<RequestId>,
        navigation_cleanup: crate::timeline::NavigationProjectionCleanup,
    ) {
        for effect in effects {
            match effect {
                AppEffect::ContinueSlidingSyncAdmission {
                    account_epoch,
                    request_id,
                    source,
                    ..
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::ContinueSlidingSyncAdmission {
                            account_epoch: *account_epoch,
                            request_id: *request_id,
                            source: *source,
                        })
                        .await;
                }
                AppEffect::RetrySlidingSyncCapabilityDiscovery {
                    account_epoch,
                    blocked_request_id,
                    request_id,
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RetrySlidingSyncCapabilityDiscovery {
                            account_epoch: *account_epoch,
                            blocked_request_id: *blocked_request_id,
                            request_id: *request_id,
                        })
                        .await;
                }
                AppEffect::ScheduleSlidingSyncCapabilityRevalidation { account_epoch } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::ScheduleSlidingSyncCapabilityRevalidation {
                            account_epoch: *account_epoch,
                        })
                        .await;
                }
                AppEffect::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result,
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SettleSlidingSyncCapabilityRevalidation {
                            account_epoch: *account_epoch,
                            request_id: *request_id,
                            result: result.clone(),
                        })
                        .await;
                }
                AppEffect::StartSync => {
                    let request_id = self.next_internal_request_id();
                    trace_runtime_sync!(
                        "effect_start_sync",
                        [
                            DiagnosticField::token("source", "actor_projection"),
                            DiagnosticField::request_id(
                                "request_id",
                                request_id.connection_id.0,
                                request_id.sequence
                            ),
                            DiagnosticField::token("action", "send_sync_start"),
                        ],
                        "source=actor_projection request_id={} action=send_sync_start",
                        runtime_request_id_trace_label(request_id)
                    );
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SyncCommand(SyncCommand::Start {
                            request_id,
                        }))
                        .await;
                }
                AppEffect::StopSync => {
                    let request_id = self.next_internal_request_id();
                    trace_runtime_sync!(
                        "effect_stop_sync",
                        [
                            DiagnosticField::token("source", "actor_projection"),
                            DiagnosticField::request_id(
                                "request_id",
                                request_id.connection_id.0,
                                request_id.sequence
                            ),
                            DiagnosticField::token("action", "send_sync_stop"),
                        ],
                        "source=actor_projection request_id={} action=send_sync_stop",
                        runtime_request_id_trace_label(request_id)
                    );
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SyncCommand(SyncCommand::Stop {
                            request_id,
                        }))
                        .await;
                }
                AppEffect::SubscribeTimeline { room_id } => {
                    if self.state.navigation.active_room_id.as_deref() != Some(room_id.as_str()) {
                        continue;
                    }
                    if self.navigation_projection_generation == 0 {
                        self.navigation_projection_generation = 1;
                    }
                    let request_id = navigation_projection_cause
                        .unwrap_or_else(|| self.next_internal_request_id());
                    let Some(account_key) = self.current_account_key() else {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::SessionRequired,
                        });
                        continue;
                    };
                    let _ = self.account_actor.admit_navigation_projection(
                        crate::timeline::NavigationProjectionIntent {
                            generation: navigation_projection_generation
                                .unwrap_or(self.navigation_projection_generation),
                            key: TimelineKey {
                                account_key,
                                kind: TimelineKind::Room {
                                    room_id: room_id.clone(),
                                },
                            },
                            cause_request_id: request_id,
                            replay_existing: true,
                            cleanup: navigation_cleanup.clone(),
                        },
                    );
                }
                AppEffect::PersistRoomPreferences { preferences, .. } => {
                    self.persist_room_preferences(preferences).await;
                }
                AppEffect::RejectProvisionalSession => {
                    let request_id = self.next_internal_request_id();
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RejectProvisionalSession { request_id })
                        .await;
                }
                AppEffect::CheckCurrentDeviceTrust => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::CheckCurrentDeviceTrust)
                        .await;
                }
                AppEffect::InspectSecureBackup => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::InspectSecureBackup)
                        .await;
                }
                AppEffect::SyncConnectivityChanged { proven } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::SyncConnectivityChanged { proven: *proven })
                        .await;
                }
                AppEffect::RefreshCurrentSessionStatus {
                    request_id,
                    trigger,
                } => {
                    let _ = self
                        .account_actor
                        .send(AccountMessage::RefreshCurrentSessionStatus {
                            request_id: *request_id,
                            trigger: *trigger,
                            sync_state: current_session_sync_state(&self.state.sync),
                        })
                        .await;
                }
                AppEffect::RestoreSession
                | AppEffect::DiscoverLogin { .. }
                | AppEffect::Login { .. }
                | AppEffect::DiscoverVerificationMethods
                | AppEffect::BeginSessionVerification { .. }
                | AppEffect::RecoverE2ee(_)
                | AppEffect::RequestVerification { .. }
                | AppEffect::AcceptVerification { .. }
                | AppEffect::ConfirmSasVerification { .. }
                | AppEffect::CancelVerification { .. }
                | AppEffect::BootstrapCrossSigning { .. }
                | AppEffect::EnableKeyBackup { .. }
                | AppEffect::RestoreKeyBackup { .. }
                | AppEffect::ResetIdentity { .. }
                | AppEffect::PersistSession(_)
                | AppEffect::PersistSettings { .. }
                | AppEffect::PaginateTimelineBackwards { .. }
                | AppEffect::SendText { .. }
                | AppEffect::OpenThreadTimeline { .. }
                | AppEffect::OpenFocusedTimeline { .. }
                | AppEffect::SearchMessages { .. }
                | AppEffect::SearchAttachments { .. }
                | AppEffect::SubscribeThreadsList { .. }
                | AppEffect::SubscribeThreadsListScoped { .. }
                | AppEffect::PaginateThreadsList { .. }
                | AppEffect::UnsubscribeThreadsList
                | AppEffect::NotifySearchCrawlerRoomsAvailable { .. }
                | AppEffect::InvalidateSearchCrawlerCache
                | AppEffect::RebuildSearchIndex
                | AppEffect::RecordNativeAttentionRecomputed { .. }
                | AppEffect::EmitUiEvent(_) => {}
            }
        }
    }

    async fn handle_ui_event_effects(&self, effects: &[AppEffect]) {
        self.handle_ui_event_effects_with_display_label_users(effects, &[])
            .await;
    }

    async fn handle_ui_event_effects_with_display_label_users(
        &self,
        effects: &[AppEffect],
        additional_user_ids: &[&str],
    ) {
        for effect in effects {
            if let AppEffect::EmitUiEvent(ui_event) = effect {
                self.handle_ui_event_effect(ui_event, additional_user_ids)
                    .await;
            } else if let AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, settings } =
                effect
            {
                // Route from actor-projection path: forward to SearchActor via
                // AccountActor (fire-and-forget, idempotent).
                let _ = self
                    .account_actor
                    .send(
                        crate::account::AccountMessage::NotifySearchCrawlerRoomsAvailable {
                            room_ids: room_ids.clone(),
                            settings: settings.clone(),
                        },
                    )
                    .await;
            } else if let AppEffect::InvalidateSearchCrawlerCache = effect {
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::InvalidateSearchCrawlerCache)
                    .await;
            } else if let AppEffect::RebuildSearchIndex = effect {
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::RebuildSearchIndex)
                    .await;
            }
        }
    }

    async fn handle_ui_event_effect(&self, ui_event: &UiEvent, additional_user_ids: &[&str]) {
        if *ui_event == UiEvent::ProfileChanged {
            self.emit_timeline_display_label_updates(additional_user_ids);
        }
        if *ui_event == UiEvent::SettingsChanged {
            self.emit_timeline_display_policy_update();
            let _ = self
                .account_actor
                .send(crate::account::AccountMessage::ReadStatePolicyChanged {
                    send_read_receipts: self.state.settings.values.notifications.send_read_receipts,
                })
                .await;
            let _ = self
                .account_actor
                .send(crate::account::AccountMessage::DisplayPolicyChanged {
                    thread_root_order: self.state.settings.values.timeline.thread_root_order,
                })
                .await;
            self.broadcast_link_preview_policy().await;
        }
        if *ui_event == UiEvent::LinkPreviewSettingsChanged {
            self.broadcast_link_preview_policy().await;
        }
    }

    async fn broadcast_link_preview_policy(&self) {
        if self.current_account_key().is_none() {
            return;
        }
        self.send_timeline_command_or_fail(
            RequestId {
                connection_id: INTERNAL_RUNTIME_CONNECTION_ID,
                sequence: 0,
            },
            TimelineCommand::BroadcastLinkPreviewPolicy {
                unencrypted_global_enabled: self.state.settings.values.display.url_previews_enabled,
                encrypted_global_enabled: self
                    .state
                    .settings
                    .values
                    .display
                    .encrypted_url_previews_enabled,
                room_overrides: self.state.link_preview_settings.room_overrides.clone(),
            },
        )
        .await;
    }

    fn emit_timeline_display_label_updates(&self, additional_user_ids: &[&str]) {
        let own_user_id = crate::event_projection::timeline_projection_own_user_id(&self.state);
        let labels = crate::event_projection::derive_display_label_updates_for_user_ids(
            &self.state.profile,
            own_user_id,
            additional_user_ids.iter().copied(),
        );
        if !labels.is_empty() {
            self.emit(CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated {
                labels,
            }));
        }
    }

    fn emit_timeline_display_policy_update(&self) {
        self.emit(CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated {
            hide_redacted: self.state.settings.values.display.hide_redacted,
        }));
    }

    async fn send_timeline_command_or_fail(&self, request_id: RequestId, command: TimelineCommand) {
        if !self
            .account_actor
            .send(AccountMessage::TimelineCommand(command))
            .await
        {
            self.emit(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::ShutdownFailed,
            });
        }
    }

    async fn ensure_room_event_cached(
        &self,
        request_id: RequestId,
        room_id: &str,
        event_id: &str,
    ) -> bool {
        let (response_tx, response_rx) = oneshot::channel();
        if !self
            .account_actor
            .send(AccountMessage::EnsureRoomEventCached {
                request_id,
                room_id: room_id.to_owned(),
                event_id: event_id.to_owned(),
                response_tx,
            })
            .await
        {
            return false;
        }
        response_rx
            .await
            .map(|r| matches!(r, crate::account::RoomEventLookupResult::Located))
            .unwrap_or(false)
    }

    fn current_account_key(&self) -> Option<AccountKey> {
        match &self.state.session {
            SessionState::Provisional { info, .. }
            | SessionState::AwaitingVerification { info, .. }
            | SessionState::Verifying { info, .. }
            | SessionState::AwaitingBootstrapConfirmation { info, .. }
            | SessionState::Rejecting { info, .. }
            | SessionState::Ready(info)
            | SessionState::Locked(info) => Some(AccountKey(info.user_id.clone())),
            SessionState::SignedOut
            | SessionState::Restoring
            | SessionState::SwitchingAccount { .. }
            | SessionState::Authenticating { .. }
            | SessionState::CapabilityBlocked { .. }
            | SessionState::LoggingOut => None,
        }
    }

    fn current_thread_timeline_key(&self) -> Option<TimelineKey> {
        let account_key = self.current_account_key()?;
        match &self.state.thread {
            ThreadPaneState::Opening {
                room_id,
                root_event_id,
                ..
            }
            | ThreadPaneState::Open {
                room_id,
                root_event_id,
                ..
            } => Some(TimelineKey {
                account_key,
                kind: TimelineKind::Thread {
                    room_id: room_id.clone(),
                    root_event_id: root_event_id.clone(),
                },
            }),
            ThreadPaneState::Closed => None,
        }
    }

    fn current_room_timeline_key(&self) -> Option<TimelineKey> {
        let account_key = self.current_account_key()?;
        let room_id = self.state.navigation.active_room_id.clone()?;
        Some(TimelineKey {
            account_key,
            kind: TimelineKind::Room { room_id },
        })
    }

    fn unsubscribe_replaced_thread_timeline(
        &self,
        room_id: &str,
        root_event_id: &str,
    ) -> Option<TimelineKey> {
        let replacement_key = TimelineKey {
            account_key: self.current_account_key()?,
            kind: TimelineKind::Thread {
                room_id: room_id.to_owned(),
                root_event_id: root_event_id.to_owned(),
            },
        };
        unsubscribe_replaced_thread_timeline_key(
            self.current_thread_timeline_key(),
            replacement_key,
        )
    }

    fn emit(&self, event: CoreEvent) {
        // A send error only means no consumer is currently attached.
        let _ = self.event_tx.send(event);
    }

    fn settle_command_admissions(&mut self) {
        let generation = self.state_generation;
        for admission in self.pending_command_admissions.drain(..) {
            let _ = admission.send(CoreCommandAdmission {
                admitted_generation: generation,
            });
        }
    }

    fn publish_state_change(&mut self, before_state: &AppState) -> u64 {
        if let Some(generation) = self.publish_state_delta(before_state) {
            return generation;
        }
        if &self.state != before_state {
            self.publish_snapshot_refresh_without_delta();
        }
        self.state_generation
    }

    /// Refresh internal snapshot-only state without inventing a StateDelta
    /// generation. Composer drafts and scheduled-send persistence are excluded
    /// from the WebView delta contract but remain observable to Core consumers.
    fn publish_snapshot_refresh_without_delta(&self) {
        let _ = self.snapshot_tx.send(VersionedAppStateSnapshot {
            generation: self.state_generation,
            state: self.state.clone(),
        });
    }

    fn publish_state_delta(&mut self, before_state: &AppState) -> Option<u64> {
        let delta = build_state_delta(self.state_generation + 1, before_state, &self.state)?;
        self.state_generation = delta.generation;
        let _ = self.snapshot_tx.send(VersionedAppStateSnapshot {
            generation: self.state_generation,
            state: self.state.clone(),
        });
        self.emit(CoreEvent::StateDelta(delta));
        Some(self.state_generation)
    }
}

fn unsubscribe_replaced_thread_timeline_key(
    current_key: Option<TimelineKey>,
    replacement_key: TimelineKey,
) -> Option<TimelineKey> {
    unsubscribe_replaced_timeline_key(current_key, replacement_key)
}

fn current_session_status_noop_event(
    request_id: RequestId,
    already_checking: bool,
    published_generation: u64,
) -> Option<CoreEvent> {
    already_checking.then_some(CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive),
        published_generation,
    })
}

fn is_ready_session_for_commands(session: &SessionState) -> bool {
    matches!(session, SessionState::Ready(_))
}

fn secure_backup_setup_projection_failure(
    state: &AppState,
    command: &AccountCommand,
) -> Option<CoreFailure> {
    let AccountCommand::BootstrapSecureBackup { request, .. } = command else {
        return None;
    };
    if matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        koushi_state::SecureBackupSetupState::SettingUp { .. }
    ) {
        return Some(CoreFailure::SecureBackupSetupFailedNoOp);
    }
    Some(match request.intent.admission(&state.secure_backup_gate) {
        SecureBackupSetupAdmission::ConfirmationRequired => {
            CoreFailure::SecureBackupSetupConfirmationRequired
        }
        SecureBackupSetupAdmission::Allowed => CoreFailure::SecureBackupSetupFailedNoOp,
        SecureBackupSetupAdmission::FailedNoOp => CoreFailure::SecureBackupSetupFailedNoOp,
    })
}

fn is_verification_gate_command(command: &CoreCommand, session: &SessionState) -> bool {
    if matches!(
        command,
        CoreCommand::Account(AccountCommand::RetryCurrentDeviceTrustDiscovery { .. })
    ) {
        return matches!(
            session,
            SessionState::Provisional {
                phase: koushi_state::ProvisionalPhase::RecheckingTrust { .. },
                ..
            } | SessionState::AwaitingVerification { .. }
        );
    }
    if matches!(
        command,
        CoreCommand::Account(
            AccountCommand::StartDeviceCleanup { .. }
                | AccountCommand::SubmitDeviceCleanupUia { .. }
                | AccountCommand::EraseDeviceCleanupLocalDataAnyway { .. }
        )
    ) {
        return matches!(
            session,
            SessionState::AwaitingVerification { .. }
                | SessionState::Provisional {
                    phase: koushi_state::ProvisionalPhase::RecheckingTrust { .. },
                    ..
                }
        );
    }
    if !matches!(
        session,
        SessionState::Provisional { .. }
            | SessionState::AwaitingVerification { .. }
            | SessionState::Verifying { .. }
            | SessionState::AwaitingBootstrapConfirmation { .. }
    ) {
        return false;
    }
    matches!(
        command,
        CoreCommand::Account(
            AccountCommand::RequestVerification { .. }
                | AccountCommand::RetryCurrentDeviceTrustDiscovery { .. }
                | AccountCommand::SubmitRecovery { .. }
                | AccountCommand::StartSessionBootstrap { .. }
                | AccountCommand::ConfirmSessionBootstrapSaved { .. }
                | AccountCommand::ResetLocalData { .. }
                | AccountCommand::StartOwnUserSas { .. }
                | AccountCommand::AcceptVerification { .. }
                | AccountCommand::ConfirmSasVerification { .. }
                | AccountCommand::CancelVerification { .. }
        )
    )
}

fn room_preferences_session_key(state: &AppState) -> Option<koushi_protocol::SessionKeyId> {
    composer_draft_session_key(state)
}

fn effects_open_thread_timeline(effects: &[AppEffect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, AppEffect::OpenThreadTimeline { .. }))
}

fn account_command_projected_action(command: &AccountCommand) -> Option<AppAction> {
    match command {
        AccountCommand::DiscoverLogin { homeserver, .. }
        | AccountCommand::StartOidcLogin { homeserver, .. } => {
            Some(AppAction::LoginDiscoveryRequested {
                homeserver: homeserver.clone(),
            })
        }
        AccountCommand::RequestVerification { request_id, target } => {
            Some(AppAction::VerificationRequested {
                request_id: request_id.sequence,
                target: target.clone(),
            })
        }
        AccountCommand::SubmitRecovery {
            request_id,
            request,
        } => Some(AppAction::E2eeRecoverySubmitted {
            flow_id: request_id.sequence,
            request: request.clone(),
        }),
        AccountCommand::StartSessionBootstrap { flow_id, .. } => {
            Some(AppAction::VerificationMethodSubmitted {
                method: koushi_state::VerificationMethod::Bootstrap,
                flow_id: *flow_id,
            })
        }
        AccountCommand::StartOwnUserSas { flow_id, .. } => {
            Some(AppAction::VerificationMethodSubmitted {
                method: koushi_state::VerificationMethod::ExistingDeviceSas,
                flow_id: *flow_id,
            })
        }
        AccountCommand::ConfirmSessionBootstrapSaved { flow_id, .. } => {
            Some(AppAction::BootstrapRecoverySavedConfirmed { flow_id: *flow_id })
        }
        AccountCommand::AcceptVerification { flow_id, .. } => {
            Some(AppAction::VerificationAccepted {
                request_id: *flow_id,
            })
        }
        AccountCommand::ConfirmSasVerification { flow_id, .. } => {
            Some(AppAction::VerificationConfirmed {
                request_id: *flow_id,
            })
        }
        AccountCommand::BootstrapCrossSigning { request_id, .. } => {
            Some(AppAction::BootstrapCrossSigningRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::EnableKeyBackup { request_id, .. } => {
            Some(AppAction::EnableKeyBackupRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::RestoreKeyBackup {
            request_id,
            version,
            ..
        } => Some(AppAction::RestoreKeyBackupRequested {
            request_id: request_id.sequence,
            version: version.clone(),
        }),
        AccountCommand::ExportRoomKeys { request_id, .. } => {
            Some(AppAction::RoomKeyExportRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::ImportRoomKeys { request_id, .. } => {
            Some(AppAction::RoomKeyImportRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::BootstrapSecureBackup {
            request_id,
            request,
        } => Some(AppAction::SecureBackupSetupRequested {
            request_id: request_id.sequence,
            intent: request.intent,
        }),
        AccountCommand::RecoverSecureBackup { .. }
        | AccountCommand::RetrySecureBackupInspection { .. } => Some(
            AppAction::SecureBackupGateChanged(koushi_state::SecureBackupGateState::Checking),
        ),
        AccountCommand::ChangeSecureBackupPassphrase { request_id, .. } => {
            Some(AppAction::SecureBackupPassphraseChangeRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::ResetIdentity { request_id } => Some(AppAction::ResetIdentityRequested {
            request_id: request_id.sequence,
        }),
        AccountCommand::CancelIdentityReset { flow_id, .. } => {
            Some(AppAction::ResetIdentityCancelled {
                request_id: *flow_id,
            })
        }
        AccountCommand::ProbeLocalEncryptionHealth { request_id } => {
            Some(AppAction::LocalEncryptionProbeRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::ResetLocalData { request_id } => Some(AppAction::ResetLocalDataRequested {
            request_id: request_id.sequence,
        }),
        AccountCommand::StartDeviceCleanup { request_id } => {
            Some(AppAction::DeviceCleanupStartRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::SubmitDeviceCleanupUia {
            request_id: _,
            flow_id,
            ..
        } => Some(AppAction::DeviceCleanupUiaSubmitted {
            request_id: *flow_id,
            flow_id: *flow_id,
        }),
        AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id } => {
            Some(AppAction::DeviceCleanupEraseLocalAnywayRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::SubmitIdentityResetAuth { flow_id, .. } => {
            Some(AppAction::ResetIdentityAuthSubmitted {
                request_id: *flow_id,
            })
        }
        AccountCommand::RefreshCurrentSessionStatus {
            request_id,
            trigger,
        } => Some(AppAction::CurrentSessionStatusRefreshRequested {
            request_id: request_id.sequence,
            trigger: *trigger,
        }),
        AccountCommand::LoadAccountManagementCapabilities { .. } => {
            Some(AppAction::AccountManagementCapabilitiesLoadRequested)
        }
        AccountCommand::ChangePassword { request_id, .. } => {
            Some(AppAction::AccountManagementRequested {
                request_id: request_id.sequence,
                operation: AccountManagementOperation::ChangePassword,
            })
        }
        AccountCommand::DeactivateAccount { request_id, .. } => {
            Some(AppAction::AccountManagementRequested {
                request_id: request_id.sequence,
                operation: AccountManagementOperation::DeactivateAccount,
            })
        }
        AccountCommand::SubmitAccountManagementUia {
            request_id: _,
            flow_id,
            ..
        } => Some(AppAction::AccountManagementAuthSubmitted {
            request_id: *flow_id,
            flow_id: *flow_id,
        }),
        AccountCommand::SoftLogoutReauth { request_id, .. } => {
            Some(AppAction::SoftLogoutReauthRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::SetDisplayName {
            request_id,
            display_name,
        } => Some(AppAction::ProfileUpdateRequested {
            request_id: request_id.sequence,
            request: ProfileUpdateRequest::SetDisplayName {
                display_name: display_name.clone(),
            },
        }),
        AccountCommand::SetLocalUserAlias {
            request_id,
            user_id,
            alias,
        } => Some(AppAction::LocalUserAliasUpdateRequested {
            request_id: request_id.sequence,
            user_id: user_id.clone(),
            alias: alias.clone(),
        }),
        AccountCommand::SetAvatar {
            request_id,
            request,
        } => Some(AppAction::ProfileUpdateRequested {
            request_id: request_id.sequence,
            request: ProfileUpdateRequest::SetAvatar {
                mime_type: request.mime_type.clone(),
                byte_count: request.bytes.len() as u64,
            },
        }),
        AccountCommand::IgnoreUser {
            request_id,
            user_id,
        } => Some(AppAction::IgnoredUserUpdateRequested {
            request_id: request_id.sequence,
            user_id: user_id.clone(),
            ignored: true,
        }),
        AccountCommand::UnignoreUser {
            request_id,
            user_id,
        } => Some(AppAction::IgnoredUserUpdateRequested {
            request_id: request_id.sequence,
            user_id: user_id.clone(),
            ignored: false,
        }),
        AccountCommand::ReportUser { .. } => None,
        AccountCommand::LoginPassword {
            request_id,
            request,
            ..
        } => Some(AppAction::AuthenticationStarted {
            attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
            homeserver: request.homeserver.clone(),
        }),
        AccountCommand::RestoreSession { .. } | AccountCommand::RestoreLastSession { .. } => {
            Some(AppAction::RestoreSessionRequested)
        }
        AccountCommand::CompleteOidcLogin { .. }
        | AccountCommand::RetrySlidingSyncCapability { .. }
        | AccountCommand::ChangeHomeserver { .. }
        | AccountCommand::QuerySavedSessions { .. }
        | AccountCommand::SetPresence { .. }
        | AccountCommand::DownloadAvatarThumbnail { .. }
        | AccountCommand::Logout { .. }
        | AccountCommand::CancelVerification { .. }
        | AccountCommand::RetryCurrentDeviceTrustDiscovery { .. }
        | AccountCommand::SwitchAccount { .. } => None,
    }
}

fn current_session_sync_state(
    sync: &koushi_state::SyncState,
) -> koushi_state::CurrentSessionSyncState {
    match sync {
        koushi_state::SyncState::Stopped => koushi_state::CurrentSessionSyncState::Stopped,
        koushi_state::SyncState::Starting => koushi_state::CurrentSessionSyncState::Starting,
        koushi_state::SyncState::Running => koushi_state::CurrentSessionSyncState::Running,
        koushi_state::SyncState::Failed { .. } | koushi_state::SyncState::Reconnecting { .. } => {
            koushi_state::CurrentSessionSyncState::Error
        }
    }
}

fn map_state_search_scope_to_core(scope: AppSearchScope) -> SearchScope {
    match scope {
        AppSearchScope::AllRooms => SearchScope::AllRooms,
        AppSearchScope::CurrentSpace { space_id } => SearchScope::CurrentSpace { space_id },
        AppSearchScope::CurrentRoom { room_id } => SearchScope::CurrentRoom { room_id },
    }
}

/// Resolve the user data directory from a `HOME` value (pure; testable).
///
/// Fails closed: there is NO current-working-directory fallback. The encrypted
/// SDK store, encrypted search index, and persisted session live under this
/// path, so silently writing them into an arbitrary CWD when `HOME` is missing
/// would be a privacy/security footgun (REPOSITORY_RULES Key Management:
/// "Missing, corrupt, or inaccessible OS secrets MUST fail closed").
fn default_data_dir_from_home(home: Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    let home =
        home.ok_or_else(|| "HOME is required to resolve koushi-desktop data dir".to_owned())?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("koushi-desktop"))
}

/// Default application data directory (`$HOME/.local/share/koushi-desktop`).
fn default_data_dir() -> PathBuf {
    default_data_dir_from_home(std::env::var_os("HOME"))
        .expect("HOME is required to resolve koushi-desktop data dir")
}

#[cfg(test)]
mod tests;
