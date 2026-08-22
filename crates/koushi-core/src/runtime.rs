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
mod scheduled_send;

pub use composer::{COMPOSER_DRAFT_PERSIST_DEBOUNCE, ForwardedComposerDraftPermit};
use composer::{
    ComposerAcceptanceIdentity, ComposerDraftLoadStatus, PendingComposerAcceptance,
    PendingComposerDraftPersist, composer_acceptance_identity_for_action,
    composer_acceptance_identity_for_timeline_command, composer_draft_acceptance_would_exhaust,
    composer_draft_account_matches, composer_draft_session_key,
    timeline_submission_revision_exhaustion,
};
use navigation::{
    NavigationPersistenceStatus, NavigationReplacementRoomForCleanup, PendingFocusedNavigation,
    anchored_action_after_projection_ack, cancel_replaced_room_timeline_link_previews_key,
    cancel_replaced_room_timeline_pagination_key, effects_open_focused_timeline,
    focused_navigation_outcome_after_reduce, navigation_replacement_room_for_cleanup,
    unsubscribe_replaced_timeline_key,
};
use scheduled_send::scheduled_send_id;

pub use connection::{CommandSubmitError, CoreCommandHandle, CoreConnection, EventStreamLag};
use std::collections::{BTreeSet, HashMap};
use std::future;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicU64};
use std::time::{Duration, Instant};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    AccountManagementOperation, ActivityRowKind, ActivityState, AppAction, AppEffect, AppState,
    ComposerDraftStore, ComposerTarget, LoginAttemptId, NavigationState, OperationFailureKind,
    ProfileUpdateRequest, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem,
    SearchScope as AppSearchScope, SessionState, SpaceMembersCommandRejection, ThreadPaneState,
    UiEvent, admit_space_member_cancellation, admit_space_member_invite, admit_space_members_load,
    reduce,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account::{AccountActorHandle, AccountMessage};
use crate::activity_resolution::ActivityResolutionRequest;
use crate::command::{
    AccountCommand, AppCommand, CoreCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand,
};
use crate::composer_draft_lifecycle::{ComposerDraftCommandPermit, ComposerDraftLeaseRegistry};
use crate::event::{
    ActivityEvent, CoreEvent, IntentNoOpReason, IntentOutcome, NativeAttentionEvent, TimelineEvent,
    VersionedAppStateSnapshot,
};
pub use activity::ACTIVITY_RECENT_MAX_ROWS;
use activity::{
    ActivityProjection, activity_tab_token, cap_activity_resolution_requests,
    guard_activity_resolution_completion, normalize_activity_resolution_action,
    record_activity_transition,
};

use crate::executor;
use crate::failure::{CoreFailure, RoomFailureKind, TimelineFailureKind};
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use crate::settings::SettingsStore;
use crate::state_delta::build_state_delta;
use crate::store::{StoreActor, session_key_id_from_info};

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
/// Inter-actor command/message inboxes (AppActor -> AccountActor ->
/// Room/Timeline actors). Sized so that forwarding a command under heavy sync
/// does not block the forwarding actor's loop.
pub const ACTOR_MESSAGE_QUEUE_CAPACITY: usize = 1024;
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
    };
    let outcome = match rejection {
        SpaceMembersCommandRejection::StaleGeneration => "stale_generation",
        SpaceMembersCommandRejection::InviteAlreadyInFlight
        | SpaceMembersCommandRejection::CancellationAlreadyInFlight
        | SpaceMembersCommandRejection::AlreadyJoined
        | SpaceMembersCommandRejection::AlreadyInvited => "duplicate",
        SpaceMembersCommandRejection::NoSelectedSpace
        | SpaceMembersCommandRejection::WrongSpace
        | SpaceMembersCommandRejection::LoadBlockedByInvite
        | SpaceMembersCommandRejection::NotInvited
        | SpaceMembersCommandRejection::NotChildRoomOnly => "rejected",
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

pub(crate) fn space_member_forward_failure_action(
    command: &crate::command::RoomCommand,
) -> Option<(RequestId, AppAction)> {
    match command {
        crate::command::RoomCommand::LoadSpaceMembers {
            request_id,
            space_id,
            generation,
        } => Some((
            *request_id,
            AppAction::SpaceMembersLoadFailed {
                request_id: request_id.sequence,
                space_id: space_id.clone(),
                generation: *generation,
                kind: OperationFailureKind::Sdk,
            },
        )),
        crate::command::RoomCommand::InviteUserToSpace {
            request_id,
            space_id,
            user_id,
            generation,
        } => Some((
            *request_id,
            AppAction::SpaceMemberInviteSettled {
                request_id: request_id.sequence,
                space_id: space_id.clone(),
                user_id: user_id.clone(),
                generation: *generation,
                outcome: koushi_state::SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
            },
        )),
        crate::command::RoomCommand::CancelSpaceInvite {
            request_id,
            space_id,
            user_id,
            generation,
        } => Some((
            *request_id,
            AppAction::SpaceMemberInviteCancellationSettled {
                request_id: request_id.sequence,
                space_id: space_id.clone(),
                user_id: user_id.clone(),
                generation: *generation,
                outcome: koushi_state::SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
            },
        )),
        _ => None,
    }
}

struct CoreCommandEnvelope {
    command: CoreCommand,
    composer_permit: Option<ComposerDraftCommandPermit>,
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
}

#[cfg(any(test, feature = "test-hooks"))]
impl ComposerDraftIoBarrierForTesting {
    pub async fn wait_for_save_started(&mut self) {
        (&mut self.save_started)
            .await
            .expect("composer draft save-start probe must remain available");
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

impl CoreRuntime {
    /// Start the runtime. Must be called within an async runtime context.
    pub fn start() -> Self {
        Self::start_with_data_dir(default_data_dir())
    }

    /// Start with a custom data directory (used by QA binaries and tests).
    pub fn start_with_data_dir(data_dir: PathBuf) -> Self {
        let account_store_actor = StoreActor::new(data_dir.clone());
        let composer_draft_store_actor = StoreActor::new(data_dir.clone());
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
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
        )
    }

    #[cfg(any(test, feature = "test-hooks", feature = "qa-bin"))]
    pub fn start_with_data_dir_and_file_credentials(
        data_dir: PathBuf,
        credential_dir: PathBuf,
    ) -> Self {
        let account_store_actor = StoreActor::with_backend(
            crate::store::CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                credential_dir.clone(),
            )),
            data_dir.clone(),
        );
        let composer_draft_store_actor = StoreActor::with_backend(
            crate::store::CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                credential_dir,
            )),
            data_dir.clone(),
        );
        Self::start_inner(
            EVENT_QUEUE_CAPACITY,
            data_dir,
            account_store_actor,
            composer_draft_store_actor,
        )
    }

    fn start_inner(
        event_capacity: usize,
        data_dir: PathBuf,
        store_actor: StoreActor,
        composer_draft_store_actor: StoreActor,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel(COMMAND_INBOX_CAPACITY);
        // NOTE: action_tx is the high-volume action-projection inbox; it must be
        // ACTION_QUEUE_CAPACITY (not COMMAND_INBOX_CAPACITY) so large-account
        // sync bursts never overflow the RoomActor's drop-on-full try_send.
        let (event_tx, _) = broadcast::channel(event_capacity);
        let (action_tx, action_rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        #[cfg(any(test, feature = "test-hooks"))]
        let (composer_draft_test_tx, composer_draft_test_rx) = mpsc::channel(1);
        let settings_store = SettingsStore::new(&data_dir);
        let composer_draft_leases = Arc::new(ComposerDraftLeaseRegistry::new());
        let sliding_sync_diagnostics = crate::SlidingSyncDiagnostics::default();
        let composer_draft_lease_changes = composer_draft_leases.subscribe();
        let (composer_draft_rejected_tx, composer_draft_rejected_rx) = mpsc::unbounded_channel();

        let mut initial_state = AppState::default();
        let settings_action = match settings_store.load() {
            Ok(values) => AppAction::SettingsLoaded { values },
            Err(_) => AppAction::SettingsLoadFailed {
                message: "settings could not be loaded".to_owned(),
            },
        };
        let _ = reduce(&mut initial_state, settings_action);
        let (snapshot_tx, snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
            generation: 0,
            state: initial_state.clone(),
        });

        // Spawn AccountActor with shared channels.
        let account_actor = crate::account::AccountActor::spawn_with_diagnostics(
            store_actor,
            action_tx.clone(),
            event_tx.clone(),
            crate::link_preview::LinkPreviewContext::from_settings(&initial_state.settings.values),
            Arc::clone(&composer_draft_leases),
            sliding_sync_diagnostics.clone(),
        );

        #[cfg(any(test, feature = "test-hooks"))]
        let account_actor_test_handle = account_actor.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let composer_draft_store_actor_for_testing = composer_draft_store_actor.clone();
        let actor = AppActor {
            command_rx,
            action_rx,
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_test_rx,
            event_tx: event_tx.clone(),
            snapshot_tx,
            state: initial_state,
            settings_store,
            composer_draft_store_actor,
            composer_draft_load_status: ComposerDraftLoadStatus::Unloaded,
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
            account_actor,
            activity_projection: ActivityProjection::default(),
            activity_resolution_generation: 0,
            next_internal_request_sequence: 1,
            navigation_projection_generation: 0,
            pending_select: HashMap::new(),
            pending_focused_navigation: None,
            pending_date_navigation_request_id: None,
        };
        let actor = executor::spawn(actor.run());
        let media_preparation =
            Arc::new(crate::media_preparation::MediaPreparationService::default());
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
            action_tx,
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_test_tx,
            media_preparation,
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
        self.composer_draft_store_actor_for_testing
            .install_composer_draft_io_probe(
                save_started_tx,
                save_release_rx,
                save_completed_tx,
                load_started_tx,
                load_completed_tx,
            );
        ComposerDraftIoBarrierForTesting {
            save_started,
            save_release: Some(save_release),
            save_completed,
            load_started,
            load_completed,
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
            action_tx: _,
            #[cfg(any(test, feature = "test-hooks"))]
                composer_draft_test_tx: _,
            media_preparation: _,
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

struct AppActor {
    command_rx: mpsc::Receiver<CoreCommandEnvelope>,
    action_rx: mpsc::Receiver<Vec<AppAction>>,
    #[cfg(any(test, feature = "test-hooks"))]
    composer_draft_test_rx: mpsc::Receiver<ComposerDraftTestMutation>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_tx: watch::Sender<VersionedAppStateSnapshot>,
    state: AppState,
    settings_store: SettingsStore,
    composer_draft_store_actor: StoreActor,
    composer_draft_load_status: ComposerDraftLoadStatus,
    navigation_loaded_for: Option<koushi_key::SessionKeyId>,
    navigation_persistence_status: NavigationPersistenceStatus,
    scheduled_sends_loaded_for: Option<koushi_key::SessionKeyId>,
    room_preferences_loaded_for: Option<koushi_key::SessionKeyId>,
    state_generation: u64,
    pending_composer_draft_persist: Option<PendingComposerDraftPersist>,
    composer_draft_leases: Arc<ComposerDraftLeaseRegistry>,
    composer_draft_lease_changes: watch::Receiver<()>,
    composer_draft_rejected_tx: mpsc::UnboundedSender<RequestId>,
    composer_draft_rejected_rx: mpsc::UnboundedReceiver<RequestId>,
    pending_composer_acceptances: HashMap<RequestId, PendingComposerAcceptance>,
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
    pending_date_navigation_request_id: Option<RequestId>,
}

enum CommandDisposition {
    Handle(CoreCommandEnvelope),
    Shutdown,
}

fn command_disposition(envelope: CoreCommandEnvelope) -> CommandDisposition {
    if matches!(
        &envelope.command,
        CoreCommand::App(AppCommand::Shutdown { .. })
    ) {
        CommandDisposition::Shutdown
    } else {
        CommandDisposition::Handle(envelope)
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
        account: &koushi_key::SessionKeyId,
        target: &ComposerTarget,
    ) -> Option<TimelineKey> {
        let account_key = crate::ids::AccountKey(account.user_id.clone());
        match target {
            ComposerTarget::Main { room_id } => {
                Some(TimelineKey::room(account_key, room_id.clone()))
            }
            ComposerTarget::Thread {
                room_id,
                root_event_id,
            } => Some(TimelineKey {
                account_key,
                kind: crate::ids::TimelineKind::Thread {
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
                        self.publish_state_delta(&before_state);
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
                        self.publish_state_delta(&before_state);
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
                    let before_state = self.state.clone();
                    let clone_ms = loop_started.elapsed().as_millis();
                    let mut state_changed = match command_disposition(command) {
                        CommandDisposition::Handle(command) => self.handle_command(command).await,
                        CommandDisposition::Shutdown => break,
                    };
                    let mut handled = 1u32;
                    let mut shutdown = false;
                    // Coalesce: drain whatever is already queued before
                    // emitting a single StateChanged for the batch. Shutdown is
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
                        self.publish_state_delta(&before_state);
                    }
                    app_loop_trace("command", handled, clone_ms, loop_started.elapsed());
                    if shutdown {
                        break;
                    }
                }
                actions = self.action_rx.recv() => {
                    let Some(actions) = actions else { break };
                    #[cfg(any(test, feature = "test-hooks"))]
                    self.apply_pending_composer_draft_test_mutations().await;
                    let loop_started = std::time::Instant::now();
                    let action_batch = actions.len() as u32;
                    let before_state = self.state.clone();
                    let clone_ms = loop_started.elapsed().as_millis();
                    let mut state_changed = false;
                    for action in actions {
                        let Some(action) = normalize_activity_resolution_action(&self.state, action)
                        else {
                            continue;
                        };
                        // Navigation must be loaded before any actor projection
                        // can persist a derived room-list order. In particular,
                        // a Sliding Sync snapshot may arrive in the same action
                        // batch as session readiness.
                        if !matches!(&action, AppAction::NavigationLoaded { .. }) {
                            self.load_navigation_for_current_session().await;
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
                        if let AppAction::ActivityRowsObserved { rows } = &action {
                            self.activity_projection.ingest(rows.clone());
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
                        // After reduce: determine outcome and emit IntentLifecycle
                        // for correlated pending SelectRoom intents.
                        if let Some((room_id, session_ready, found, already, rooms_len)) =
                            select_intent_pre
                        {
                            let committed = self
                                .state
                                .navigation
                                .active_room_id
                                .as_deref()
                                == Some(room_id.as_str());
                            let outcome = if !session_ready {
                                IntentOutcome::FailedNoOp(IntentNoOpReason::SessionNotReady)
                            } else if !found {
                                IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState)
                            } else if already {
                                IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive)
                            } else if committed {
                                IntentOutcome::Committed
                            } else {
                                // Room was present, session ready, but reduce
                                // did not commit — classify as FailedNoOp to
                                // prevent a silent timeout (defensive case).
                                IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState)
                            };
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
                            let request_id_to_emit = self
                                .pending_select
                                .get_mut(&room_id)
                                .and_then(|q| q.pop_front());
                            if self
                                .pending_select
                                .get(&room_id)
                                .map(|q| q.is_empty())
                                .unwrap_or(false)
                            {
                                self.pending_select.remove(&room_id);
                            }
                            if let Some(request_id) = request_id_to_emit {
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
                                    .field(DiagnosticField::token(
                                        "outcome",
                                        intent_outcome_token(&outcome),
                                    )),
                                );
                                self.emit(CoreEvent::IntentLifecycle { request_id, outcome });
                            }
                            if committed {
                                navigation_projection_cause = request_id_to_emit;
                            }
                        }
                        self.handle_post_projection_effects(
                            &post_projection_effects,
                            navigation_projection_generation,
                            navigation_projection_cause,
                            crate::timeline::NavigationProjectionCleanup {
                                cancel_pagination:
                                    cancel_replaced_room_timeline_pagination,
                                cancel_link_previews:
                                    cancel_replaced_room_timeline_link_previews,
                            },
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
                        if let Some(activity_update) = self
                            .activity_projection
                            .update_action_for_open_state(&self.state)
                        {
                            let _activity_effects =
                                self.reduce_app_action(activity_update).await;
                        }
                        self.handle_ui_event_effects(&post_projection_effects).await;
                        self.load_room_preferences_for_current_session().await;
                        self.load_navigation_for_current_session().await;
                        self.load_composer_drafts_for_current_session().await;
                        self.load_scheduled_sends_for_current_session().await;
                        state_changed = true;
                    }
                    if state_changed {
                        self.publish_state_delta(&before_state);
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
    async fn apply_pending_composer_draft_test_mutations(&mut self) {
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
            let _ = mutation.completion.send(self.state.clone());
        }
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

    /// Returns whether `AppState` changed.
    async fn handle_command(&mut self, envelope: CoreCommandEnvelope) -> bool {
        let CoreCommandEnvelope {
            command,
            composer_permit,
        } = envelope;
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
            return false;
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
                    return projected_state_changed;
                }
                self.handle_ui_event_effects_with_display_label_users(
                    &effects,
                    &display_label_user_ids,
                )
                .await;
                let requires_projection_acceptance = matches!(
                    &account_command,
                    AccountCommand::RestoreSession { .. }
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
                    self.emit(CoreEvent::OperationFailed {
                        request_id: command_request_id,
                        failure: CoreFailure::SessionRequired,
                    });
                    return false;
                }
                // Route to AccountActor; it will produce AppActions and
                // CoreEvents. AppActor does not immediately know the result —
                // it observes it via the action channel.
                let _ = self
                    .account_actor
                    .send(AccountMessage::Command(account_command))
                    .await;
                projected_state_changed
            }
            CoreCommand::App(app_command) => match app_command {
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
                        crate::timeline::validate_composer_body_for_timeline_send(&body)
                    {
                        if kind == TimelineFailureKind::UnsupportedSlashCommand
                            && let Some(key) =
                                self.composer_target_notice_key(&expected_account, &target)
                        {
                            self.emit(CoreEvent::Room(
                                crate::event::RoomEvent::ComposerSlashCommandRejected {
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
                        crate::timeline::validate_composer_body_for_timeline_send(&body)
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
                                crate::event::RoomEvent::ComposerSlashCommandRejected {
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
                    self.ensure_room_event_cached(request_id, &room_id, &event_id)
                        .await;
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
                    self.ensure_room_event_cached(request_id, &room_id, &event_id)
                        .await;
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
                AppCommand::AcknowledgeTimelineProjection {
                    request_id: _,
                    projection_request_id,
                    key,
                    generation,
                    item_count,
                    target_present,
                } => {
                    let pending_navigation = self
                        .pending_focused_navigation
                        .as_ref()
                        .filter(|pending| {
                            pending.projection_request_id == projection_request_id
                                && pending.key == key
                        })
                        .cloned();
                    let pending_matches = pending_navigation.is_some();
                    let (response, accepted) = oneshot::channel();
                    let routed = self
                        .account_actor
                        .send(AccountMessage::AcknowledgeTimelineProjection {
                            projection_request_id,
                            key: key.clone(),
                            generation,
                            response,
                        })
                        .await;
                    let actor_acknowledgement = if routed {
                        accepted.await.unwrap_or_default()
                    } else {
                        crate::timeline::TimelineProjectionAcknowledgement::default()
                    };
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Debug,
                            "core.activity_navigation",
                            "projection_acknowledged",
                        )
                        .field(DiagnosticField::count("item_count", item_count))
                        .field(DiagnosticField::boolean(
                            "frontend_target_present",
                            target_present,
                        ))
                        .field(DiagnosticField::count(
                            "actor_item_count",
                            actor_acknowledgement.item_count,
                        ))
                        .field(DiagnosticField::boolean(
                            "actor_target_present",
                            actor_acknowledgement.target_present,
                        ))
                        .field(DiagnosticField::boolean(
                            "evidence_matches",
                            target_present == actor_acknowledgement.target_present
                                && item_count == actor_acknowledgement.item_count,
                        ))
                        .field(DiagnosticField::boolean(
                            "actor_accepted",
                            actor_acknowledgement.accepted,
                        )),
                    );
                    if pending_matches
                        && let Some(action) = anchored_action_after_projection_ack(
                            &mut self.pending_focused_navigation,
                            projection_request_id,
                            &key,
                            actor_acknowledgement.accepted,
                            target_present,
                            actor_acknowledgement.target_present,
                        )
                    {
                        let navigation = pending_navigation
                            .expect("matching focused navigation must remain available");
                        let target_found =
                            matches!(action, AppAction::EnterAnchoredTimeline { .. });
                        let outcome = if target_found {
                            "anchor_committed"
                        } else {
                            "live_fallback"
                        };
                        record(DiagnosticEvent::new(
                            DiagnosticLevel::Debug,
                            "core.activity_navigation",
                            outcome,
                        ));
                        let focused_key = (!target_found)
                            .then(|| self.current_focused_context_timeline_key())
                            .flatten();
                        let effects = self.reduce_app_action(action).await;
                        if let Some(key) = focused_key {
                            self.send_timeline_command_or_fail(
                                projection_request_id,
                                TimelineCommand::Unsubscribe {
                                    request_id: projection_request_id,
                                    key,
                                },
                            )
                            .await;
                        }
                        self.handle_app_effects(projection_request_id, effects)
                            .await;
                        let lifecycle_outcome = focused_navigation_outcome_after_reduce(
                            &self.state,
                            &navigation,
                            target_found,
                        );
                        self.emit(CoreEvent::IntentLifecycle {
                            request_id: projection_request_id,
                            outcome: lifecycle_outcome,
                        });
                    }
                    true
                }
                AppCommand::AcknowledgeTimelineBatchRendered {
                    request_id,
                    key,
                    actor_generation,
                    timeline_generation,
                    repair_generation,
                    batch_id,
                } => {
                    if !self
                        .account_actor
                        .send(AccountMessage::AcknowledgeTimelineBatchRendered {
                            key,
                            actor_generation,
                            timeline_generation,
                            repair_generation,
                            batch_id,
                        })
                        .await
                    {
                        self.emit(CoreEvent::OperationFailed {
                            request_id,
                            failure: CoreFailure::ShutdownFailed,
                        });
                    }
                    false
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
                                crate::command::RoomCommand::MarkRoomAsRead {
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
                    crate::command::RoomCommand::LoadSpaceMembers {
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
                    crate::command::RoomCommand::InviteUserToSpace {
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
                    crate::command::RoomCommand::CancelSpaceInvite {
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
                    _ => {}
                }
                // User-intent lane: for SelectRoom, record the request_id→room_id
                // correlation BEFORE forwarding so the action loop can emit the
                // terminal IntentLifecycle outcome. This command path is reliable
                // and must never be converted into a drop-on-full background path.
                if let crate::command::RoomCommand::SelectRoom {
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
                    timeline_command.composer_account_fence()
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
                    let request_id = timeline_command
                        .composer_account_fence()
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
                                scope: map_core_search_scope_to_state(scope.clone()),
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
        command: &crate::command::TimelineCommand,
    ) -> bool {
        match command {
            crate::command::TimelineCommand::SendReadReceipt { .. } => {
                !self.state.settings.values.notifications.send_read_receipts
            }
            crate::command::TimelineCommand::SetTyping { .. } => {
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
                            crate::command::ThreadsListCommand::Open {
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
                            crate::command::ThreadsListCommand::Open {
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
                            crate::command::ThreadsListCommand::Paginate {
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
                            crate::command::ThreadsListCommand::Close { request_id },
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
        let own_user_id = crate::event::timeline_projection_own_user_id(&self.state);
        let labels = crate::event::derive_display_label_updates_for_user_ids(
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

    async fn ensure_room_event_cached(&self, request_id: RequestId, room_id: &str, event_id: &str) {
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
            return;
        }
        let _ = response_rx.await;
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

    fn publish_state_delta(&mut self, before_state: &AppState) {
        let Some(delta) = build_state_delta(self.state_generation + 1, before_state, &self.state)
        else {
            return;
        };
        self.state_generation = delta.generation;
        let _ = self.snapshot_tx.send(VersionedAppStateSnapshot {
            generation: self.state_generation,
            state: self.state.clone(),
        });
        self.emit(CoreEvent::StateDelta(delta));
        // Legacy compatibility for core/headless consumers that still wait on
        // full snapshots. The Tauri webview adapter ignores this event on the
        // normal state path and applies StateDelta instead.
        self.emit(CoreEvent::StateChanged(self.state.clone()));
    }
}

fn unsubscribe_replaced_thread_timeline_key(
    current_key: Option<TimelineKey>,
    replacement_key: TimelineKey,
) -> Option<TimelineKey> {
    unsubscribe_replaced_timeline_key(current_key, replacement_key)
}

fn is_ready_session_for_commands(session: &SessionState) -> bool {
    matches!(session, SessionState::Ready(_))
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

fn room_preferences_session_key(state: &AppState) -> Option<koushi_key::SessionKeyId> {
    composer_draft_session_key(state)
}

fn effects_open_thread_timeline(effects: &[AppEffect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, AppEffect::OpenThreadTimeline { .. }))
}

fn map_core_search_scope_to_state(scope: SearchScope) -> AppSearchScope {
    match scope {
        SearchScope::AllRooms => AppSearchScope::AllRooms,
        SearchScope::CurrentRoom { room_id } => AppSearchScope::CurrentRoom { room_id },
        SearchScope::CurrentSpace { space_id } => AppSearchScope::CurrentSpace { space_id },
    }
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
        AccountCommand::BootstrapSecureBackup { request_id, .. } => {
            Some(AppAction::SecureBackupSetupRequested {
                request_id: request_id.sequence,
            })
        }
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
        AccountCommand::QueryDevices { request_id } => {
            Some(AppAction::DeviceSessionsLoadRequested {
                request_id: request_id.sequence,
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
        AccountCommand::RenameDevice { request_id, .. } => {
            Some(AppAction::AccountManagementRequested {
                request_id: request_id.sequence,
                operation: AccountManagementOperation::RenameDevice,
            })
        }
        AccountCommand::DeleteDevices {
            request_id,
            device_ordinals,
            ..
        } => Some(AppAction::AccountManagementRequested {
            request_id: request_id.sequence,
            operation: if device_ordinals.len() == 1 {
                AccountManagementOperation::DeleteDevice
            } else {
                AccountManagementOperation::DeleteOtherDevices
            },
        }),
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
        #[cfg(feature = "qa-bin")]
        AccountCommand::QaSetLocalDeviceBlacklisted { .. }
        | AccountCommand::QaRefreshDeviceKeysAndAssertKnown { .. } => None,
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
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::event::{AccountEvent, RoomEvent, TimelineEvent};
    use koushi_state::{
        DisplaySettings, RoomSummary, RoomTags, SessionInfo, SettingsPatch, SpaceMemberEntry,
        SpaceMemberMembership, SpaceMembersProjection, UserProfile,
    };

    fn closed_forward_space_member_entry(
        user_id: &str,
        membership: SpaceMemberMembership,
    ) -> SpaceMemberEntry {
        SpaceMemberEntry {
            user_id: user_id.to_owned(),
            display_name: Some("Closed forward test user".to_owned()),
            display_label: "Closed forward test user".to_owned(),
            original_display_label: "Closed forward test user".to_owned(),
            avatar_url: None,
            power_level: Some(0),
            role: koushi_state::RoomMemberRole::User,
            membership,
            child_room_ids: Vec::new(),
            invite_pending: false,
        }
    }

    fn closed_forward_space_member_fixture(
        space_id: &str,
        generation: u64,
        user_id: &str,
        membership: SpaceMemberMembership,
    ) -> Vec<AppAction> {
        let entry = closed_forward_space_member_entry(user_id, membership);
        let (space_joined, space_invited, child_room_only) = match membership {
            SpaceMemberMembership::SpaceJoined => (vec![entry], Vec::new(), Vec::new()),
            SpaceMemberMembership::SpaceInvited => (Vec::new(), vec![entry], Vec::new()),
            SpaceMemberMembership::ChildRoomOnly => (Vec::new(), Vec::new(), vec![entry]),
        };
        vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@closed-forward-self:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
            AppAction::NavigationLoaded {
                navigation: NavigationState {
                    active_space_id: Some(space_id.to_owned()),
                    ..NavigationState::default()
                },
            },
            AppAction::SpaceMembersLoadRequested {
                request_id: 1,
                space_id: space_id.to_owned(),
                generation,
            },
            AppAction::SpaceMembersLoaded {
                request_id: 1,
                projection: SpaceMembersProjection {
                    space_id: space_id.to_owned(),
                    generation,
                    space_joined,
                    space_invited,
                    child_room_only,
                    child_room_count: 0,
                    complete_child_room_count: 0,
                    incomplete_child_room_count: 0,
                },
            },
        ]
    }

    async fn wait_for_runtime_snapshot(
        connection: &mut CoreConnection,
        predicate: impl Fn(&AppState) -> bool,
    ) -> AppState {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = connection.snapshot();
                if predicate(&snapshot) {
                    return snapshot;
                }
                let _ = connection
                    .recv_event()
                    .await
                    .expect("runtime event stream should remain open");
            }
        })
        .await
        .expect("runtime state should reach the expected operation boundary")
    }

    async fn close_account_actor_for_runtime_test(runtime: &CoreRuntime) {
        let (acknowledged_tx, acknowledged_rx) = oneshot::channel();
        assert!(
            runtime
                .account_actor_test_handle
                .send(AccountMessage::ShutdownWithAck {
                    acknowledged: acknowledged_tx,
                })
                .await
        );
        acknowledged_rx
            .await
            .expect("AccountActor closed-channel test acknowledgement");
    }

    async fn run_closed_space_member_forwarding_case(
        membership: SpaceMemberMembership,
        command: impl FnOnce(RequestId) -> crate::command::RoomCommand,
    ) -> (AppState, CoreFailure, u64) {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let runtime = CoreRuntime::start_with_event_capacity(64);
        let mut connection = runtime.attach();
        let space_id = "!closed-forward-space:example.invalid";
        let user_id = "@closed-forward-user:example.invalid";
        let generation = 9;

        runtime
            .inject_actions(closed_forward_space_member_fixture(
                space_id, generation, user_id, membership,
            ))
            .await;
        wait_for_runtime_snapshot(&mut connection, |snapshot| {
            snapshot.space_members.selected_space_id.as_deref() == Some(space_id)
                && snapshot.space_members.generation == generation
                && matches!(
                    snapshot.space_members.operation,
                    koushi_state::SpaceMembersOperationState::Idle
                )
        })
        .await;

        close_account_actor_for_runtime_test(&runtime).await;
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Room(command(request_id)))
            .await
            .expect("closed-channel command should enter AppActor");

        let failure = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match connection
                    .recv_event()
                    .await
                    .expect("runtime event stream should remain open")
                {
                    CoreEvent::OperationFailed {
                        request_id: failed_request_id,
                        failure,
                    } if failed_request_id == request_id => break failure,
                    _ => {}
                }
            }
        })
        .await
        .expect("closed actor forwarding should emit a correlated failure");
        let final_state = wait_for_runtime_snapshot(&mut connection, |snapshot| {
            matches!(
                snapshot.space_members.operation,
                koushi_state::SpaceMembersOperationState::Failed {
                    request_id: failed_request_id,
                    ..
                } if failed_request_id == request_id.sequence
            )
        })
        .await;

        let debug = format!("{:?}", final_state.space_members);
        let diagnostics = serde_json::to_string(&koushi_diagnostics::snapshot())
            .expect("diagnostics should serialize");
        for private_value in [space_id, user_id] {
            assert!(!debug.contains(private_value), "{debug}");
            assert!(!diagnostics.contains(private_value), "{diagnostics}");
        }

        runtime.shutdown_handle().abort();
        runtime.media_lifecycle.abort();
        (final_state, failure, request_id.sequence)
    }

    #[tokio::test]
    async fn closed_account_forwarding_rolls_back_space_member_load() {
        let (state, failure, request_id) = run_closed_space_member_forwarding_case(
            SpaceMemberMembership::SpaceJoined,
            |request_id| crate::command::RoomCommand::LoadSpaceMembers {
                request_id,
                space_id: "!closed-forward-space:example.invalid".to_owned(),
                generation: 9,
            },
        )
        .await;

        assert_eq!(
            failure,
            CoreFailure::RoomOperationFailed {
                kind: RoomFailureKind::Sdk
            }
        );
        assert!(matches!(
            state.space_members.operation,
            koushi_state::SpaceMembersOperationState::Failed {
                request_id: failed_request_id,
                user_id: None,
                kind: OperationFailureKind::Sdk,
                ..
            } if failed_request_id == request_id
        ));
    }

    #[tokio::test]
    async fn closed_account_forwarding_rolls_back_optimistic_space_invite() {
        let (state, failure, request_id) = run_closed_space_member_forwarding_case(
            SpaceMemberMembership::ChildRoomOnly,
            |request_id| crate::command::RoomCommand::InviteUserToSpace {
                request_id,
                space_id: "!closed-forward-space:example.invalid".to_owned(),
                user_id: "@closed-forward-user:example.invalid".to_owned(),
                generation: 9,
            },
        )
        .await;

        assert_eq!(
            failure,
            CoreFailure::RoomOperationFailed {
                kind: RoomFailureKind::Sdk
            }
        );
        assert!(
            state
                .space_members
                .child_room_only
                .iter()
                .any(|entry| entry.user_id == "@closed-forward-user:example.invalid")
        );
        assert!(state.space_members.space_invited.is_empty());
        assert!(matches!(
            state.space_members.operation,
            koushi_state::SpaceMembersOperationState::Failed {
                request_id: failed_request_id,
                user_id: Some(ref failed_user_id),
                kind: OperationFailureKind::Sdk,
                ..
            } if failed_request_id == request_id
                && failed_user_id == "@closed-forward-user:example.invalid"
        ));
    }

    #[tokio::test]
    async fn closed_account_forwarding_retains_invited_row_for_cancellation_retry() {
        let (state, failure, request_id) = run_closed_space_member_forwarding_case(
            SpaceMemberMembership::SpaceInvited,
            |request_id| crate::command::RoomCommand::CancelSpaceInvite {
                request_id,
                space_id: "!closed-forward-space:example.invalid".to_owned(),
                user_id: "@closed-forward-user:example.invalid".to_owned(),
                generation: 9,
            },
        )
        .await;

        assert_eq!(
            failure,
            CoreFailure::RoomOperationFailed {
                kind: RoomFailureKind::Sdk
            }
        );
        assert!(
            state
                .space_members
                .space_invited
                .iter()
                .any(|entry| entry.user_id == "@closed-forward-user:example.invalid")
        );
        assert!(matches!(
            state.space_members.operation,
            koushi_state::SpaceMembersOperationState::Failed {
                request_id: failed_request_id,
                user_id: Some(ref failed_user_id),
                kind: OperationFailureKind::Sdk,
                ..
            } if failed_request_id == request_id
                && failed_user_id == "@closed-forward-user:example.invalid"
        ));
    }

    pub(super) fn unread_diagnostic_room(room_id: &str) -> RoomSummary {
        RoomSummary {
            room_id: room_id.to_owned(),
            display_name: "Synthetic room".to_owned(),
            display_label: "Synthetic room".to_owned(),
            original_display_label: "Synthetic room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 2,
            highlight_count: 1,
            marked_unread: true,
            recency_stamp: Some(42),
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }
    }

    #[test]
    fn app_loop_trace_ignores_subthreshold_iterations() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let before = koushi_diagnostics::snapshot();
        app_loop_trace("test_boundary", 1, 2, Duration::from_millis(99));
        let after = koushi_diagnostics::snapshot();
        assert_eq!(
            after
                .records
                .iter()
                .filter(|record| record.event.source == "core.runtime"
                    && record.event.stage == "app_loop")
                .count(),
            before
                .records
                .iter()
                .filter(|record| record.event.source == "core.runtime"
                    && record.event.stage == "app_loop")
                .count()
        );
    }

    #[test]
    fn app_loop_trace_records_at_threshold_without_environment_switch() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let before = koushi_diagnostics::snapshot();
        app_loop_trace("test_boundary", 3, 4, Duration::from_millis(100));
        let after = koushi_diagnostics::snapshot();
        assert!(after.records.len() > before.records.len());
        let record = after
            .records
            .iter()
            .rev()
            .find(|record| {
                record.event.source == "core.runtime" && record.event.stage == "app_loop"
            })
            .expect("threshold iteration should be collected");
        assert!(record.event.fields.iter().any(|field| field.key == "count"));
    }

    #[test]
    fn default_data_dir_requires_home() {
        assert!(default_data_dir_from_home(None).is_err());
    }

    #[test]
    fn default_data_dir_uses_xdg_like_user_data_path() {
        let dir = default_data_dir_from_home(Some("/tmp/synthetic-home".into())).unwrap();
        assert!(dir.ends_with(".local/share/koushi-desktop"));
    }

    #[test]
    fn search_scope_round_trips_non_all_scope_kinds() {
        let source = include_str!("runtime.rs");
        let to_state = source
            .split("fn map_core_search_scope_to_state")
            .nth(1)
            .expect("core-to-state search scope mapper should exist")
            .split("fn account_command_projected_action")
            .next()
            .expect("account command projector should follow search scope mapper");
        let to_core = source
            .split("fn map_state_search_scope_to_core")
            .nth(1)
            .expect("state-to-core search scope mapper should exist")
            .split("fn default_data_dir_from_home")
            .next()
            .expect("data-dir helper should follow search scope mapper");

        assert!(
            to_state.contains("SearchScope::CurrentRoom")
                && to_state.contains("SearchScope::CurrentSpace")
                && to_state.contains("SearchScope::AllRooms"),
            "core search scopes must preserve Room/DM, current-space, and all-rooms kinds in AppState"
        );
        assert!(
            to_core.contains("AppSearchScope::CurrentRoom")
                && to_core.contains("AppSearchScope::CurrentSpace")
                && to_core.contains("AppSearchScope::AllRooms"),
            "submitted AppState search scopes must round-trip through core without collapsing to global"
        );
    }

    #[test]
    fn activity_mark_read_routes_persistent_room_mark_read_commands() {
        let source = include_str!("runtime.rs");
        let branch = source
            .split("AppCommand::MarkActivityRead")
            .nth(1)
            .expect("MarkActivityRead branch should exist")
            .split("AppCommand::OpenFilesView")
            .next()
            .expect("OpenFilesView should follow MarkActivityRead");

        assert!(
            branch.contains("RoomCommand::MarkRoomAsRead"),
            "Activity mark-read must persist room unread state through RoomActor, not only mutate local projection"
        );
        assert!(
            branch.contains("next_internal_request_id"),
            "Activity mark-read persistence must use internal correlated requests"
        );
        assert!(
            branch.contains("FullyReadMarkerUpdated"),
            "Activity mark-read should still update the local marker after selecting the persistent event ids"
        );
    }

    #[tokio::test]
    async fn versioned_snapshot_generation_matches_state_delta_generation() {
        let runtime = CoreRuntime::start_with_event_capacity(8);
        let mut connection = runtime.attach();

        runtime
            .inject_actions(vec![
                AppAction::AppStarted,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),
                    user_id: "@me:example.invalid".to_owned(),
                    device_id: "DEVICE".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;

        let mut delta = None;
        for _ in 0..8 {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                    .await
                    .expect("runtime should emit state delta")
                    .expect("event stream should stay open");
            if let CoreEvent::StateDelta(next) = event {
                delta = Some(next);
                break;
            }
        }
        let delta = delta.expect("expected state delta event");

        let snapshot = connection.versioned_snapshot();
        assert_eq!(snapshot.generation, delta.generation);
        assert_eq!(snapshot.generation, 1);
        assert!(matches!(
            snapshot.state.session,
            koushi_state::SessionState::Ready(_)
        ));
        runtime.shutdown_handle().abort();
    }

    #[tokio::test]
    async fn rejected_space_invites_are_fenced_before_room_actor_route() {
        let runtime = CoreRuntime::start_with_event_capacity(64);
        let mut connection = runtime.attach();
        let space_id = "!space-a:example.invalid".to_owned();
        let duplicate_user_id = "@duplicate:example.invalid".to_owned();
        let generation = 7;

        runtime
            .inject_actions(vec![
                AppAction::AppStarted,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),
                    user_id: "@me:example.invalid".to_owned(),
                    device_id: "DEVICE".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
                AppAction::SpaceMembersLoadRequested {
                    request_id: 1,
                    space_id: space_id.clone(),
                    generation,
                },
                AppAction::SpaceMembersLoaded {
                    request_id: 1,
                    projection: SpaceMembersProjection {
                        space_id: space_id.clone(),
                        generation,
                        space_joined: Vec::new(),
                        space_invited: vec![SpaceMemberEntry {
                            user_id: duplicate_user_id.clone(),
                            display_name: None,
                            display_label: "Unknown user".to_owned(),
                            original_display_label: "Unknown user".to_owned(),
                            avatar_url: None,
                            power_level: None,
                            role: koushi_state::RoomMemberRole::User,
                            membership: SpaceMemberMembership::SpaceInvited,
                            child_room_ids: Vec::new(),
                            invite_pending: false,
                        }],
                        child_room_only: Vec::new(),
                        child_room_count: 0,
                        complete_child_room_count: 0,
                        incomplete_child_room_count: 0,
                    },
                },
            ])
            .await;

        let expected_state = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = connection.snapshot();
                if snapshot.space_members.selected_space_id.as_deref() == Some(space_id.as_str())
                    && snapshot.space_members.generation == generation
                    && snapshot.space_members.space_invited.len() == 1
                {
                    break snapshot;
                }
                let _ = connection.recv_event().await.expect("runtime event stream");
            }
        })
        .await
        .expect("injected Space member state should settle");

        let rejected_commands = [
            (
                "wrong_space",
                "!space-b:example.invalid".to_owned(),
                generation,
            ),
            ("stale_generation", space_id.clone(), generation + 1),
            ("duplicate", space_id.clone(), generation),
        ];
        for (reason, target_space_id, target_generation) in rejected_commands {
            let request_id = connection.next_request_id();
            connection
                .command(CoreCommand::Room(
                    crate::command::RoomCommand::InviteUserToSpace {
                        request_id,
                        space_id: target_space_id,
                        user_id: duplicate_user_id.clone(),
                        generation: target_generation,
                    },
                ))
                .await
                .expect("rejected invite command should enter the runtime");

            let failure = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    match connection.recv_event().await.expect("runtime event stream") {
                        CoreEvent::OperationFailed {
                            request_id: failed_request_id,
                            failure,
                        } if failed_request_id == request_id => break failure,
                        CoreEvent::Room(RoomEvent::SpaceMemberInviteSettled { .. }) => {
                            panic!("{reason} invite reached RoomActor settlement route")
                        }
                        _ => {}
                    }
                }
            })
            .await
            .expect("rejected invite should emit a correlated failure");
            assert_eq!(
                failure,
                CoreFailure::RoomOperationFailed {
                    kind: crate::failure::RoomFailureKind::Sdk,
                }
            );
            assert_eq!(connection.snapshot(), expected_state);
        }

        let no_settlement = tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let CoreEvent::Room(RoomEvent::SpaceMemberInviteSettled { .. }) =
                    connection.recv_event().await.expect("runtime event stream")
                {
                    return true;
                }
            }
        })
        .await;
        assert!(
            no_settlement.is_err(),
            "no rejected invite should reach the RoomActor/SDK settlement path"
        );
        runtime.shutdown_handle().abort();
    }

    #[tokio::test]
    async fn projection_rejected_restore_emits_one_correlated_failure_without_routing() {
        let runtime = CoreRuntime::start_with_event_capacity(16);
        let mut connection = runtime.attach();
        runtime
            .inject_actions(vec![AppAction::LogoutRequested])
            .await;

        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), connection.recv_event())
                .await
                .expect("logout projection should be published")
                .expect("event stream should remain open");
            if matches!(
                event,
                CoreEvent::StateChanged(AppState {
                    session: SessionState::LoggingOut,
                    ..
                })
            ) {
                break;
            }
        }

        let restore_request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Account(AccountCommand::RestoreSession {
                request_id: restore_request_id,
                account_key: AccountKey("@restore-rejected:example.invalid".to_owned()),
            }))
            .await
            .expect("restore command should enter the bounded runtime inbox");
        let marker_request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Account(AccountCommand::QuerySavedSessions {
                request_id: marker_request_id,
            }))
            .await
            .expect("ordered marker should enter the bounded runtime inbox");

        let mut restore_failure_count = 0;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), connection.recv_event())
                .await
                .expect("projection rejection should settle before the ordered marker")
                .expect("event stream should remain open");
            match event {
                CoreEvent::OperationFailed {
                    request_id,
                    failure: CoreFailure::SessionRequired,
                } if request_id == restore_request_id => {
                    restore_failure_count += 1;
                }
                CoreEvent::OperationFailed { request_id, .. }
                    if request_id == restore_request_id =>
                {
                    panic!("projection rejection emitted the wrong failure kind")
                }
                CoreEvent::Account(AccountEvent::SessionRestored { request_id, .. })
                    if request_id == restore_request_id =>
                {
                    panic!("projection-rejected restore was routed to AccountActor")
                }
                CoreEvent::Account(AccountEvent::SavedSessionsListed { request_id, .. })
                    if request_id == marker_request_id =>
                {
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(
            restore_failure_count, 1,
            "a projection-rejected command must have exactly one terminal failure"
        );
        assert!(matches!(
            connection.snapshot().session,
            SessionState::LoggingOut
        ));
        runtime.shutdown_handle().abort();
    }

    #[tokio::test]
    async fn actor_profile_changes_emit_timeline_display_label_updates() {
        let runtime = CoreRuntime::start_with_event_capacity(8);
        let mut connection = runtime.attach();

        runtime
            .inject_actions(vec![
                AppAction::AppStarted,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),
                    user_id: "@me:example.invalid".to_owned(),
                    device_id: "DEVICE".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
                AppAction::UserProfilesUpdated {
                    profiles: vec![UserProfile {
                        user_id: "@alice:example.invalid".to_owned(),
                        display_name: Some("Alice Upstream".to_owned()),
                        display_label: String::new(),
                        original_display_label: String::new(),
                        mention_search_terms: Vec::new(),
                        avatar: None,
                    }],
                },
                AppAction::LocalUserAliasesLoaded {
                    aliases: BTreeMap::from([(
                        "@alice:example.invalid".to_owned(),
                        "Alice Alias".to_owned(),
                    )]),
                },
            ])
            .await;

        let mut saw_alias_update = false;
        for _ in 0..4 {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                    .await
                    .expect("runtime should emit profile/timeline events")
                    .expect("event stream should stay open");
            if let CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated { labels }) = event
                && labels.iter().any(|label| {
                    label.user_id == "@alice:example.invalid"
                        && label.display_label == "Alice Alias"
                })
            {
                saw_alias_update = true;
                break;
            }
        }

        assert!(
            saw_alias_update,
            "actor-origin ProfileChanged effects must relabel already-loaded timeline rows"
        );
        runtime.shutdown_handle().abort();
    }

    #[tokio::test]
    async fn settings_update_emits_timeline_display_policy_update() {
        let runtime = CoreRuntime::start_with_event_capacity(16);
        let mut connection = runtime.attach();

        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(
                crate::command::AppCommand::UpdateSettings {
                    request_id,
                    patch: SettingsPatch {
                        display: Some(DisplaySettings {
                            code_block_wrap: true,
                            hide_redacted: true,
                            url_previews_enabled: true,
                            encrypted_url_previews_enabled: false,
                        }),
                        ..SettingsPatch::default()
                    },
                },
            ))
            .await
            .expect("settings update command should be accepted");

        let mut saw_policy_update = false;
        for _ in 0..4 {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                    .await
                    .expect("runtime should emit settings/timeline events")
                    .expect("event stream should stay open");
            if let CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated { hide_redacted }) =
                event
            {
                saw_policy_update = hide_redacted;
                break;
            }
        }

        assert!(
            saw_policy_update,
            "SettingsChanged must reproject already-loaded redacted timeline rows"
        );
        runtime.shutdown_handle().abort();
    }

    #[tokio::test]
    async fn local_alias_clear_command_emits_target_display_label_update() {
        let runtime = CoreRuntime::start_with_event_capacity(16);
        let mut connection = runtime.attach();
        let user_id = "@unknown:example.invalid";

        runtime
            .inject_actions(vec![
                AppAction::AppStarted,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),
                    user_id: "@me:example.invalid".to_owned(),
                    device_id: "DEVICE".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
                AppAction::LocalUserAliasesLoaded {
                    aliases: BTreeMap::from([(user_id.to_owned(), "Unknown Alias".to_owned())]),
                },
            ])
            .await;

        for _ in 0..4 {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                    .await
                    .expect("runtime should emit initial profile events")
                    .expect("event stream should stay open");
            if matches!(event, CoreEvent::StateChanged(_)) {
                break;
            }
        }

        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Account(AccountCommand::SetLocalUserAlias {
                request_id,
                user_id: user_id.to_owned(),
                alias: None,
            }))
            .await
            .expect("alias clear command should be accepted");

        let mut saw_clear_update = false;
        for _ in 0..4 {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(1), connection.recv_event())
                    .await
                    .expect("runtime should emit alias-clear events")
                    .expect("event stream should stay open");
            if let CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated { labels }) = event
                && labels
                    .iter()
                    .any(|label| label.user_id == user_id && label.display_label == user_id)
            {
                saw_clear_update = true;
                break;
            }
        }

        assert!(
            saw_clear_update,
            "alias clear must relabel rows even when the target user is absent from profile.users"
        );
        runtime.shutdown_handle().abort();
    }

    #[test]
    fn open_thread_command_must_execute_thread_timeline_effects() {
        let source = include_str!("runtime.rs");
        let open_thread_arm = source
            .split("AppCommand::OpenThread")
            .nth(1)
            .expect("OpenThread arm should exist")
            .split("AppCommand::CloseThread")
            .next()
            .expect("CloseThread arm should follow OpenThread");

        assert!(
            !open_thread_arm.contains("let _ = effects;"),
            "OpenThread reducer effects are production behavior and must not be discarded"
        );
        assert!(
            open_thread_arm.contains("handle_app_effects")
                || open_thread_arm.contains("TimelineCommand::Subscribe"),
            "OpenThread must execute the OpenThreadTimeline effect through the timeline actor"
        );
    }

    #[test]
    fn runtime_must_execute_start_sync_effects_from_session_reducer() {
        let source = include_str!("runtime.rs");
        let effects_helper = source
            .split("async fn handle_app_effects")
            .nth(1)
            .expect("handle_app_effects should exist");

        assert!(
            effects_helper.contains("AppEffect::StartSync"),
            "login, restore, and E2EE recovery reducers emit StartSync; runtime must execute it"
        );
        assert!(
            effects_helper.contains("SyncCommand::Start"),
            "StartSync effects must route the canonical SyncCommand::Start path"
        );
    }

    #[test]
    fn runtime_must_execute_session_cleanup_effects_from_session_reducer() {
        let source = include_str!("runtime.rs");
        let command_effects = source
            .split("async fn handle_app_effects")
            .nth(1)
            .expect("handle_app_effects should exist")
            .split("async fn handle_post_projection_effects")
            .next()
            .expect("handle_app_effects should precede post projection effects");
        let actor_projection_effects = source
            .split("async fn handle_post_projection_effects")
            .nth(1)
            .expect("handle_post_projection_effects should exist")
            .split("async fn handle_ui_event_effects")
            .next()
            .expect("post projection effects should precede ui event effects");

        for helper in [command_effects, actor_projection_effects] {
            assert!(
                helper.contains("AppEffect::StopSync"),
                "session lock/logout reducers emit StopSync; runtime must handle it explicitly"
            );
            assert!(
                helper.contains("SyncCommand::Stop"),
                "StopSync effects must route the canonical SyncCommand::Stop path"
            );
        }
    }

    #[test]
    fn runtime_routes_current_device_trust_rechecks_in_both_effect_lanes() {
        let source = include_str!("runtime.rs");
        let command_effects = source
            .split("async fn handle_app_effects")
            .nth(1)
            .expect("handle_app_effects should exist")
            .split("async fn handle_post_projection_effects")
            .next()
            .expect("handle_app_effects should precede post projection effects");
        let actor_projection_effects = source
            .split("async fn handle_post_projection_effects")
            .nth(1)
            .expect("handle_post_projection_effects should exist")
            .split("async fn handle_ui_event_effects")
            .next()
            .expect("post projection effects should precede ui event effects");

        for helper in [command_effects, actor_projection_effects] {
            let recheck_arm = helper
                .split("AppEffect::CheckCurrentDeviceTrust")
                .nth(1)
                .expect("trust recheck effect should be matched explicitly")
                .split("AppEffect::")
                .next()
                .expect("another effect arm should bound the trust recheck route");
            assert!(
                recheck_arm.contains("AccountMessage::CheckCurrentDeviceTrust"),
                "trust recheck effects must reach the AccountActor instead of being discarded"
            );
        }
    }

    #[test]
    fn runtime_routes_current_session_status_in_both_effect_lanes() {
        let source = include_str!("runtime.rs");
        let command_effects = source
            .split("async fn handle_app_effects")
            .nth(1)
            .expect("handle_app_effects should exist")
            .split("async fn handle_post_projection_effects")
            .next()
            .expect("handle_app_effects should precede post projection effects");
        let actor_projection_effects = source
            .split("async fn handle_post_projection_effects")
            .nth(1)
            .expect("handle_post_projection_effects should exist")
            .split("async fn handle_ui_event_effects")
            .next()
            .expect("post projection effects should precede ui event effects");

        for helper in [command_effects, actor_projection_effects] {
            let refresh_arm = helper
                .split("AppEffect::RefreshCurrentSessionStatus")
                .nth(1)
                .expect("session-status refresh effect should be matched explicitly")
                .split("AppEffect::")
                .next()
                .expect("another effect arm should bound the session-status route");
            assert!(
                refresh_arm.contains("AccountMessage::RefreshCurrentSessionStatus"),
                "session-status refresh effects must reach AccountActor instead of being discarded"
            );
        }
    }

    #[test]
    fn current_session_status_account_command_projects_open_and_manual_refreshes() {
        for trigger in [
            koushi_state::SessionStatusRefreshTrigger::Open,
            koushi_state::SessionStatusRefreshTrigger::Manual,
        ] {
            assert_eq!(
                account_command_projected_action(&AccountCommand::RefreshCurrentSessionStatus {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(2),
                        sequence: 17,
                    },
                    trigger,
                }),
                Some(AppAction::CurrentSessionStatusRefreshRequested {
                    request_id: 17,
                    trigger,
                })
            );
        }
    }

    #[test]
    fn app_actor_persistence_uses_blocking_store_port() {
        let source = include_str!("runtime.rs");
        let scheduled_source = include_str!("runtime/scheduled_send.rs");
        let load_scheduled = scheduled_source
            .split("async fn load_scheduled_sends_for_current_session")
            .nth(1)
            .expect("scheduled loader should exist")
            .split("async fn persist_scheduled_sends")
            .next()
            .expect("scheduled persist helper should follow scheduled loader");
        let save_scheduled = scheduled_source
            .split("async fn persist_scheduled_sends")
            .nth(1)
            .expect("scheduled persist helper should exist")
            .split("fn scheduled_send_delay")
            .next()
            .expect("scheduled delay should follow scheduled persist");
        let navigation_source = include_str!("runtime/navigation.rs");
        let navigation_load = navigation_source
            .split("async fn load_navigation_for_current_session")
            .nth(1)
            .expect("navigation loader should exist")
            .split("async fn persist_navigation")
            .next()
            .expect("navigation persist should follow navigation loader");
        let navigation_save = navigation_source
            .split("async fn persist_navigation")
            .nth(1)
            .expect("navigation persist helper should exist")
            .split("fn current_focused_context_timeline_key")
            .next()
            .expect("focused projection should follow navigation persist");
        let save_preferences = source
            .split("async fn persist_room_preferences")
            .nth(1)
            .expect("room preference persist helper should exist")
            .split("fn next_internal_request_id")
            .next()
            .expect("internal request ID should follow room preference persist");
        let composer_source = include_str!("runtime/composer.rs");
        let flush_drafts = composer_source
            .split("async fn flush_pending_composer_drafts")
            .nth(1)
            .expect("composer draft flush should exist")
            .split("fn composer_draft_session_key")
            .next()
            .expect("composer draft session key should follow composer draft flush");
        let settings_effect = source
            .split("AppEffect::PersistSettings")
            .nth(1)
            .expect("settings persist effect should exist")
            .split("AppEffect::PersistRoomPreferences")
            .next()
            .expect("room preference effect should follow settings effect");

        for section in [load_scheduled, save_scheduled, save_preferences] {
            assert!(
                section.contains("executor::spawn_blocking"),
                "AppActor store persistence must be offloaded from the reducer loop"
            );
        }
        for section in [navigation_load, navigation_save] {
            assert!(
                section.contains("executor::spawn_blocking"),
                "navigation store persistence must be offloaded from the reducer loop"
            );
        }
        for section in [flush_drafts, settings_effect] {
            assert!(
                section.contains("executor::spawn_blocking"),
                "AppActor store persistence must be offloaded from the reducer loop"
            );
        }
    }

    #[test]
    fn runtime_must_execute_subscribe_timeline_effects_from_navigation_reducers() {
        let source = include_str!("runtime.rs");
        let effects_helper = source
            .split("async fn handle_app_effects")
            .nth(1)
            .expect("handle_app_effects should exist");

        assert!(
            effects_helper.contains("AppEffect::SubscribeTimeline"),
            "room-list and navigation reducers emit SubscribeTimeline; runtime must execute it"
        );
        assert!(
            effects_helper.contains("TimelineKind::Room"),
            "SubscribeTimeline effects must route the canonical room timeline subscription"
        );
    }

    #[test]
    fn runtime_room_selection_replays_existing_room_timeline_for_empty_renderer_store() {
        let source = include_str!("runtime.rs");
        let effects_helper = source
            .split("async fn handle_post_projection_effects")
            .nth(1)
            .expect("post-projection effects helper should exist");

        assert!(
            effects_helper.contains("NavigationProjectionIntent"),
            "room selection should admit the latest desired room projection"
        );
        assert!(
            effects_helper.contains("admit_navigation_projection"),
            "room selection must use the bounded latest-desired projection lane"
        );
        assert!(
            effects_helper.contains("replay_existing: true"),
            "room selection must replay InitialItems from an existing actor so a rebuilt or reloaded renderer can populate an empty timeline store"
        );
    }

    #[test]
    fn closed_account_actor_timeline_route_is_not_reported_as_queue_overflow() {
        let source = include_str!("runtime.rs");
        let helper = source
            .split("async fn send_timeline_command_or_fail")
            .nth(1)
            .expect("timeline command routing helper should exist")
            .split("fn default_data_dir_from_home")
            .next()
            .expect("helper should precede utility functions");

        assert!(
            helper.contains("CoreFailure::ShutdownFailed"),
            "a closed AccountActor command route is runtime shutdown/closed, not bounded queue overflow"
        );
        assert!(
            !helper.contains("TimelineFailureKind::QueueOverflow"),
            "QueueOverflow is reserved for bounded queue backpressure/relay overflow, not closed actor routes"
        );
    }

    #[test]
    fn actor_projection_start_sync_effects_must_not_be_discarded() {
        let source = include_str!("runtime.rs");
        let action_rx_arm = source
            .split("actions = self.action_rx.recv()")
            .nth(1)
            .expect("action_rx arm should exist")
            .split("command = self.command_rx.recv()")
            .next()
            .expect("action_rx arm should be bounded");

        assert!(
            action_rx_arm.contains("handle_post_projection_effects"),
            "actor-originated LoginSucceeded/RecoverySucceeded actions emit StartSync; action_rx must execute that follow-up effect"
        );
    }

    #[test]
    fn runtime_sync_trace_covers_start_sync_effect_boundaries() {
        let source = include_str!("runtime.rs");
        let command_effects = source
            .split("async fn handle_app_effects")
            .nth(1)
            .expect("handle_app_effects should exist")
            .split("async fn handle_post_projection_effects")
            .next()
            .expect("handle_app_effects should precede post projection effects");
        let actor_projection_effects = source
            .split("async fn handle_post_projection_effects")
            .nth(1)
            .expect("handle_post_projection_effects should exist")
            .split("async fn handle_ui_event_effects")
            .next()
            .expect("post projection effects should precede ui event effects");
        let compact_command_effects: String = command_effects
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let compact_actor_projection_effects: String = actor_projection_effects
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();

        assert!(
            compact_command_effects
                .contains("trace_runtime_sync!(\"effect_start_sync\",[DiagnosticField::token(\"source\",\"command_effect\")"),
            "command-originated StartSync effects should be visible in sync diagnostics"
        );
        assert!(
            compact_actor_projection_effects
                .contains("trace_runtime_sync!(\"effect_start_sync\",[DiagnosticField::token(\"source\",\"actor_projection\")"),
            "actor-originated restore/login StartSync effects should be visible in sync diagnostics"
        );
    }

    #[test]
    fn replacement_thread_helper_preserves_same_key_and_unsubscribes_different_key() {
        let account_key = AccountKey("@alice:example.invalid".to_owned());
        let current = TimelineKey {
            account_key: account_key.clone(),
            kind: TimelineKind::Thread {
                room_id: "!room:example.invalid".to_owned(),
                root_event_id: "$root-a:example.invalid".to_owned(),
            },
        };
        let same = current.clone();
        let different = TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id: "!room:example.invalid".to_owned(),
                root_event_id: "$root-b:example.invalid".to_owned(),
            },
        };

        assert_eq!(
            unsubscribe_replaced_thread_timeline_key(Some(current.clone()), same),
            None
        );
        assert_eq!(
            unsubscribe_replaced_thread_timeline_key(Some(current.clone()), different),
            Some(current)
        );
        assert_eq!(
            unsubscribe_replaced_thread_timeline_key(None, thread_key("$root-c:example.invalid")),
            None
        );
    }

    #[test]
    fn opening_a_replacement_thread_unsubscribes_the_previous_thread_before_subscribe() {
        let source = include_str!("runtime.rs");
        let open_thread_arm = source
            .split("AppCommand::OpenThread")
            .nth(1)
            .expect("OpenThread arm should exist")
            .split("AppCommand::CloseThread")
            .next()
            .expect("CloseThread arm should follow OpenThread");

        let replacement_offset = open_thread_arm
            .find("unsubscribe_replaced_thread_timeline")
            .expect("OpenThread must check whether an existing thread timeline is being replaced");
        let effects_offset = open_thread_arm
            .find("handle_app_effects")
            .expect("OpenThread must execute the new thread subscribe effect");

        assert!(
            replacement_offset < effects_offset,
            "OpenThread must unsubscribe a different existing thread before subscribing the replacement"
        );
    }

    #[test]
    fn opening_a_replacement_focused_context_unsubscribes_previous_focused_before_subscribe() {
        let source = include_str!("runtime.rs");
        let open_focused_arm = source
            .split("AppCommand::OpenFocusedContext")
            .nth(1)
            .expect("OpenFocusedContext arm should exist")
            .split("AppCommand::CloseFocusedContext")
            .next()
            .expect("CloseFocusedContext arm should follow OpenFocusedContext");

        let replacement_offset = open_focused_arm
            .find("unsubscribe_replaced_focused_context_timeline")
            .expect(
                "OpenFocusedContext must check whether an existing focused timeline is being replaced",
            );
        let effects_offset = open_focused_arm
            .find("handle_app_effects")
            .expect("OpenFocusedContext must execute the new focused subscribe effect");

        assert!(
            replacement_offset < effects_offset,
            "OpenFocusedContext must unsubscribe a different existing focused timeline before subscribing the replacement"
        );
    }

    #[test]
    fn opening_focused_context_repairs_target_event_cache_before_subscribe() {
        let source = include_str!("runtime.rs");
        let open_focused_arm = source
            .split("AppCommand::OpenFocusedContext")
            .nth(1)
            .expect("OpenFocusedContext arm should exist")
            .split("AppCommand::CloseFocusedContext")
            .next()
            .expect("CloseFocusedContext arm should follow OpenFocusedContext");

        let repair_offset = open_focused_arm
            .find("ensure_room_event_cached")
            .expect("OpenFocusedContext must repair the target event cache before subscribing");
        let effects_offset = open_focused_arm
            .find("handle_app_effects")
            .expect("OpenFocusedContext must execute the new focused subscribe effect");

        assert!(
            repair_offset < effects_offset,
            "target event cache repair must run before focused timeline subscription effects"
        );
    }

    #[test]
    fn selecting_a_replacement_room_cancels_previous_room_pagination_before_subscribe() {
        let source = include_str!("runtime.rs");
        let action_rx_arm = source
            .split("actions = self.action_rx.recv()")
            .nth(1)
            .expect("action_rx arm should exist")
            .split("if state_changed")
            .next()
            .expect("action_rx arm should include post-reduce effect handling");

        let cancel_offset = action_rx_arm
            .find("cancel_replaced_room_timeline_pagination")
            .expect("SelectRoom must cancel in-flight pagination for the previous room timeline");
        let effects_offset = action_rx_arm
            .find("handle_post_projection_effects")
            .expect("SelectRoom must still execute SubscribeTimeline effects");

        assert!(
            cancel_offset < effects_offset,
            "room switch pagination cancellation must happen before subscribing/rendering the replacement room"
        );
        assert!(
            source.contains("TimelineCommand::CancelPagination"),
            "runtime must route room-switch pagination cancellation through the timeline actor"
        );
    }

    #[test]
    fn selecting_a_replacement_room_cancels_previous_room_link_previews_before_subscribe() {
        let source = include_str!("runtime.rs");
        let action_rx_arm = source
            .split("actions = self.action_rx.recv()")
            .nth(1)
            .expect("action_rx arm should exist")
            .split("if state_changed")
            .next()
            .expect("action_rx arm should include post-reduce effect handling");

        let cancel_offset = action_rx_arm
            .find("cancel_replaced_room_timeline_link_previews")
            .expect(
                "SelectRoom must cancel in-flight link previews for the previous room timeline",
            );
        let effects_offset = action_rx_arm
            .find("handle_post_projection_effects")
            .expect("SelectRoom must still execute SubscribeTimeline effects");

        assert!(
            cancel_offset < effects_offset,
            "room switch link preview cancellation must happen before subscribing/rendering the replacement room"
        );
        assert!(
            source.contains("TimelineCommand::CancelLinkPreviews"),
            "runtime must route room-switch link preview cancellation through the timeline actor"
        );
    }

    #[tokio::test]
    async fn committed_room_cleanup_bypasses_a_saturated_account_mailbox() {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let (account_tx, mut saturated_account_rx) = mpsc::channel(1);
        account_tx
            .try_send(AccountMessage::CancelActivityResolution)
            .expect("fill the ordinary AccountActor mailbox");
        let (navigation_projection, navigation_projection_rx) =
            crate::timeline::NavigationProjectionIngress::channel();
        drop(navigation_projection_rx);
        let account_actor =
            AccountActorHandle::for_app_actor_test(account_tx, navigation_projection.clone());

        let session = SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        let session_key = session_key_id_from_info(&session);
        let old_room = "!old:example.invalid";
        let next_room = "!next:example.invalid";
        let mut state = AppState {
            session: SessionState::Ready(session),
            rooms: vec![
                unread_diagnostic_room(old_room),
                unread_diagnostic_room(next_room),
            ],
            ..AppState::default()
        };
        state.navigation.active_room_id = Some(old_room.to_owned());

        let (command_tx, command_rx) = mpsc::channel(1);
        let (action_tx, action_rx) = mpsc::channel(1);
        let (_composer_draft_test_tx, composer_draft_test_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let (snapshot_tx, mut snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
            generation: 0,
            state: state.clone(),
        });
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(91),
            sequence: 7,
        };
        let mut pending_select = HashMap::new();
        pending_select.insert(
            next_room.to_owned(),
            std::collections::VecDeque::from([request_id]),
        );
        let composer_draft_leases = Arc::new(ComposerDraftLeaseRegistry::new());
        let composer_draft_lease_changes = composer_draft_leases.subscribe();
        let (composer_draft_rejected_tx, composer_draft_rejected_rx) = mpsc::unbounded_channel();
        let actor = AppActor {
            command_rx,
            action_rx,
            composer_draft_test_rx,
            event_tx,
            snapshot_tx,
            state,
            settings_store: SettingsStore::new(data_dir.path()),
            composer_draft_store_actor: StoreActor::new(data_dir.path().to_owned()),
            composer_draft_load_status: ComposerDraftLoadStatus::Loaded(session_key.clone()),
            navigation_loaded_for: Some(session_key.clone()),
            navigation_persistence_status: NavigationPersistenceStatus::Loaded(session_key.clone()),
            scheduled_sends_loaded_for: Some(session_key.clone()),
            room_preferences_loaded_for: Some(session_key),
            state_generation: 0,
            pending_composer_draft_persist: None,
            composer_draft_leases,
            composer_draft_lease_changes,
            composer_draft_rejected_tx,
            composer_draft_rejected_rx,
            pending_composer_acceptances: HashMap::new(),
            account_actor,
            activity_projection: ActivityProjection::default(),
            activity_resolution_generation: 0,
            next_internal_request_sequence: 1,
            navigation_projection_generation: 0,
            pending_select,
            pending_focused_navigation: None,
            pending_date_navigation_request_id: None,
        };
        let actor_task = executor::spawn(actor.run());

        action_tx
            .send(vec![AppAction::SelectRoom {
                room_id: next_room.to_owned(),
            }])
            .await
            .expect("inject committed room selection");

        let terminal = executor::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("committed terminal must not wait for cleanup transport")
            .expect("event stream remains open");
        assert!(matches!(
            terminal,
            CoreEvent::IntentLifecycle {
                request_id: observed,
                outcome: IntentOutcome::Committed,
            } if observed == request_id
        ));
        executor::timeout(Duration::from_millis(100), snapshot_rx.changed())
            .await
            .expect("the committed selection must finish reducing")
            .expect("snapshot channel remains open");
        assert_eq!(
            snapshot_rx
                .borrow()
                .state
                .navigation
                .active_room_id
                .as_deref(),
            Some(next_room)
        );
        let terminal_deadline = Instant::now() + Duration::from_millis(20);
        while let Ok(Ok(event)) = executor::timeout(
            terminal_deadline.saturating_duration_since(Instant::now()),
            event_rx.recv(),
        )
        .await
        {
            assert!(
                !matches!(
                    event,
                    CoreEvent::IntentLifecycle {
                        request_id: observed,
                        ..
                    } | CoreEvent::OperationFailed {
                        request_id: observed,
                        ..
                    } if observed == request_id
                ),
                "cleanup admission must not emit a second correlated terminal"
            );
            if Instant::now() >= terminal_deadline {
                break;
            }
        }
        let mut retained_rx = navigation_projection.subscribe();
        let retained = retained_rx
            .borrow_and_update()
            .clone()
            .expect("cleanup and replacement projection remain latest-wins");
        assert_eq!(
            retained.cleanup.cancel_pagination,
            Some(TimelineKey::room(
                AccountKey("@synthetic:example.invalid".to_owned()),
                old_room,
            ))
        );
        assert_eq!(
            retained.cleanup.cancel_link_previews,
            retained.cleanup.cancel_pagination
        );

        actor_task.abort();
        drop(command_tx);
        drop(action_tx);
        assert!(
            saturated_account_rx.try_recv().is_ok(),
            "ordinary mailbox remained saturated throughout the selection"
        );
    }

    #[test]
    fn timestamp_jump_uses_local_activity_projection_before_homeserver_fallback() {
        let source = include_str!("runtime.rs");
        let timestamp_arm = source
            .split("AppCommand::OpenTimelineAtTimestamp")
            .nth(1)
            .expect("OpenTimelineAtTimestamp arm should exist")
            .split("AppCommand::CloseFocusedContext")
            .next()
            .expect("CloseFocusedContext arm should follow OpenTimelineAtTimestamp");

        let local_projection_offset = timestamp_arm
            .find("activity_projection")
            .expect("timestamp jump must check the Rust-owned local activity projection");
        let account_fallback_offset = timestamp_arm
            .find("AccountMessage::OpenTimelineAtTimestamp")
            .expect("timestamp jump must keep the homeserver fallback");

        assert!(
            local_projection_offset < account_fallback_offset,
            "local projection resolution must run before the homeserver timestamp_to_event fallback"
        );
        assert!(
            timestamp_arm.contains("AppAction::OpenFocusedContext"),
            "local timestamp resolution must still open focused context through the reducer"
        );
    }

    #[test]
    fn identity_reset_auth_command_projects_pending_state_before_routing() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 7,
        };
        let flow_id = 99;

        assert_eq!(
            account_command_projected_action(&AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request: koushi_state::IdentityResetAuthRequest::OAuthApproved,
            }),
            Some(AppAction::ResetIdentityAuthSubmitted {
                request_id: flow_id
            })
        );
    }

    #[test]
    fn oidc_completion_has_no_speculative_appactor_projection() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 8,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::CompleteOidcLogin {
                request_id,
                callback_url: "koushi-desktop://auth/callback?code=secret".to_owned(),
                platform: koushi_state::DisplayPlatform::Linux,
            }),
            None
        );
    }

    #[test]
    fn change_homeserver_has_no_speculative_app_projection() {
        let command = AccountCommand::ChangeHomeserver {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(4),
                sequence: 12,
            },
        };

        assert_eq!(account_command_projected_action(&command), None);
    }

    #[test]
    fn oidc_authorization_start_only_projects_discovery() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 7,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::StartOidcLogin {
                request_id,
                homeserver: "https://matrix.example.org".to_owned(),
            }),
            Some(AppAction::LoginDiscoveryRequested {
                homeserver: "https://matrix.example.org".to_owned(),
            })
        );
    }

    #[test]
    fn restore_key_backup_command_projects_state_without_recovery_secret() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 9,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::RestoreKeyBackup {
                request_id,
                version: Some("backup-version-1".to_owned()),
                request: koushi_state::RecoveryRequest {
                    secret: koushi_state::AuthSecret::new("recovery secret"),
                },
            }),
            Some(AppAction::RestoreKeyBackupRequested {
                request_id: 9,
                version: Some("backup-version-1".to_owned()),
            })
        );
    }

    #[test]
    fn reset_local_data_command_projects_resetting_state_before_routing() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 17,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::ResetLocalData { request_id }),
            Some(AppAction::ResetLocalDataRequested { request_id: 17 })
        );
    }

    #[test]
    fn device_cleanup_commands_project_correlated_pending_state_before_routing() {
        let start_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 21,
        };
        let submit_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 22,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::StartDeviceCleanup {
                request_id: start_request_id,
            }),
            Some(AppAction::DeviceCleanupStartRequested { request_id: 21 })
        );
        assert_eq!(
            account_command_projected_action(&AccountCommand::SubmitDeviceCleanupUia {
                request_id: submit_request_id,
                flow_id: 21,
                password: koushi_state::AuthSecret::new("private-password"),
            }),
            Some(AppAction::DeviceCleanupUiaSubmitted {
                request_id: 21,
                flow_id: 21,
            })
        );
        assert_eq!(
            account_command_projected_action(&AccountCommand::EraseDeviceCleanupLocalDataAnyway {
                request_id: submit_request_id,
            },),
            Some(AppAction::DeviceCleanupEraseLocalAnywayRequested { request_id: 22 })
        );
    }

    #[test]
    fn device_cleanup_commands_are_admitted_from_the_provisional_gate() {
        let command = CoreCommand::Account(AccountCommand::StartDeviceCleanup {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 23,
            },
        });
        let session = SessionState::AwaitingVerification {
            info: koushi_state::SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@user:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
            gate: koushi_state::VerificationGateState {
                methods: vec![],
                account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
                failure: Some(koushi_state::VerificationGateFailureKind::Sdk),
            },
        };

        assert!(is_verification_gate_command(&command, &session));
    }

    #[test]
    fn profile_commands_project_pending_state_without_display_name_or_avatar_bytes() {
        let display_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 13,
        };
        let avatar_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 14,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::SetDisplayName {
                request_id: display_request_id,
                display_name: Some("Private Display".to_owned()),
            }),
            Some(AppAction::ProfileUpdateRequested {
                request_id: 13,
                request: ProfileUpdateRequest::SetDisplayName {
                    display_name: Some("Private Display".to_owned()),
                },
            })
        );

        assert_eq!(
            account_command_projected_action(&AccountCommand::SetAvatar {
                request_id: avatar_request_id,
                request: crate::command::SetAvatarRequest {
                    mime_type: "image/png".to_owned(),
                    bytes: vec![1, 2, 3, 4],
                },
            }),
            Some(AppAction::ProfileUpdateRequested {
                request_id: 14,
                request: ProfileUpdateRequest::SetAvatar {
                    mime_type: "image/png".to_owned(),
                    byte_count: 4,
                },
            })
        );
    }

    #[test]
    fn local_user_alias_command_projects_pending_state_without_leaking_alias() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 15,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::SetLocalUserAlias {
                request_id,
                user_id: "@private:example.invalid".to_owned(),
                alias: Some("Private Alias".to_owned()),
            }),
            Some(AppAction::LocalUserAliasUpdateRequested {
                request_id: 15,
                user_id: "@private:example.invalid".to_owned(),
                alias: Some("Private Alias".to_owned()),
            })
        );
    }

    #[test]
    fn verification_followup_commands_project_flow_id_without_speculative_cancel() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 9,
        };
        let flow_id = 42;

        assert_eq!(
            account_command_projected_action(&AccountCommand::AcceptVerification {
                request_id,
                flow_id,
            }),
            Some(AppAction::VerificationAccepted {
                request_id: flow_id,
            })
        );
        assert_eq!(
            account_command_projected_action(&AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            }),
            Some(AppAction::VerificationConfirmed {
                request_id: flow_id,
            })
        );
        assert_eq!(
            account_command_projected_action(&AccountCommand::CancelVerification {
                request_id,
                flow_id,
                reason: koushi_state::VerificationCancelReason::User,
            }),
            None
        );
    }

    #[cfg(feature = "qa-bin")]
    #[test]
    fn qa_device_key_refresh_has_no_speculative_app_projection() {
        let (acknowledged, _ack) = tokio::sync::oneshot::channel();
        assert!(
            account_command_projected_action(&AccountCommand::QaRefreshDeviceKeysAndAssertKnown {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 43,
                },
                target: koushi_state::VerificationTarget {
                    user_id: "@private:example.invalid".to_owned(),
                    device_id: "PRIVATEDEVICE".to_owned(),
                },
                acknowledged,
            })
            .is_none()
        );
    }

    #[test]
    fn trust_discovery_retry_is_admitted_only_in_retryable_gate_states() {
        let command = CoreCommand::Account(AccountCommand::RetryCurrentDeviceTrustDiscovery {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 77,
            },
        });
        let info = SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@me:example.invalid".into(),
            device_id: "DEVICE".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        let gate = koushi_state::VerificationGateState {
            methods: vec![],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: Some(koushi_state::VerificationGateFailureKind::Network),
        };
        assert!(is_verification_gate_command(
            &command,
            &SessionState::Provisional {
                info: info.clone(),
                phase: koushi_state::ProvisionalPhase::RecheckingTrust {
                    failure: Some(koushi_state::VerificationGateFailureKind::Network)
                }
            }
        ));
        assert!(is_verification_gate_command(
            &command,
            &SessionState::AwaitingVerification {
                info: info.clone(),
                gate: gate.clone()
            }
        ));
        assert!(!is_verification_gate_command(
            &command,
            &SessionState::Verifying {
                info,
                gate,
                method: koushi_state::VerificationMethod::RecoveryKey,
                flow_id: 77,
                sas_emojis: vec![]
            }
        ));
    }

    #[test]
    fn local_data_reset_is_admitted_through_the_verification_gate() {
        let command = CoreCommand::Account(AccountCommand::ResetLocalData {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 78,
            },
        });
        let info = SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@me:example.invalid".into(),
            device_id: "DEVICE".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        let gate = koushi_state::VerificationGateState {
            methods: vec![],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: Some(koushi_state::VerificationGateFailureKind::Sdk),
        };

        assert!(is_verification_gate_command(
            &command,
            &SessionState::Provisional {
                info: info.clone(),
                phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
            }
        ));
        assert!(is_verification_gate_command(
            &command,
            &SessionState::AwaitingVerification {
                info: info.clone(),
                gate: gate.clone(),
            }
        ));
        assert!(is_verification_gate_command(
            &command,
            &SessionState::Verifying {
                info,
                gate,
                method: koushi_state::VerificationMethod::RecoveryKey,
                flow_id: 78,
                sas_emojis: vec![],
            }
        ));
        assert!(!is_verification_gate_command(
            &command,
            &SessionState::SignedOut
        ));
    }

    #[test]
    fn device_cleanup_is_not_admitted_while_verification_owns_the_gate() {
        let command = CoreCommand::Account(AccountCommand::StartDeviceCleanup {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 79,
            },
        });
        let info = SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@me:example.invalid".into(),
            device_id: "DEVICE".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        let gate = koushi_state::VerificationGateState {
            methods: vec![koushi_state::VerificationMethodCapability::RecoveryKey],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: Some(koushi_state::VerificationGateFailureKind::Sdk),
        };

        assert!(is_verification_gate_command(
            &command,
            &SessionState::AwaitingVerification {
                info: info.clone(),
                gate: gate.clone(),
            }
        ));
        assert!(!is_verification_gate_command(
            &command,
            &SessionState::Verifying {
                info,
                gate,
                method: koushi_state::VerificationMethod::RecoveryKey,
                flow_id: 79,
                sas_emojis: vec![],
            }
        ));
    }

    #[test]
    fn gate_sas_and_bootstrap_commands_project_only_opaque_flow_state() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(5),
            sequence: 90,
        };
        assert_eq!(
            account_command_projected_action(&AccountCommand::StartOwnUserSas {
                request_id,
                flow_id: 31,
            }),
            Some(AppAction::VerificationMethodSubmitted {
                method: koushi_state::VerificationMethod::ExistingDeviceSas,
                flow_id: 31,
            })
        );
        assert_eq!(
            account_command_projected_action(&AccountCommand::ConfirmSessionBootstrapSaved {
                request_id,
                flow_id: 32,
            }),
            Some(AppAction::BootstrapRecoverySavedConfirmed { flow_id: 32 })
        );
        let debug = format!(
            "{:?}",
            AccountCommand::StartOwnUserSas {
                request_id,
                flow_id: 31,
            }
        );
        assert!(!debug.contains('@'));
        assert!(!debug.contains("DEVICE"));
        let bootstrap_debug = format!(
            "{:?}",
            AccountCommand::StartSessionBootstrap {
                request_id,
                flow_id: 32,
                auth: Some(koushi_state::AuthSecret::new("private-auth")),
                request: crate::command::SecureBackupSetupRequest {
                    passphrase: Some(koushi_state::AuthSecret::new("private-passphrase")),
                    recovery_key_destination_path: Some(std::path::PathBuf::from(
                        "/private/recovery-key.txt",
                    )),
                    explicit_reenable_confirmed: false,
                },
            }
        );
        for forbidden in ["private-auth", "private-passphrase", "/private/"] {
            assert!(!bootstrap_debug.contains(forbidden));
        }
    }

    fn thread_key(root_event_id: &str) -> TimelineKey {
        TimelineKey {
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            kind: TimelineKind::Thread {
                room_id: "!room:example.invalid".to_owned(),
                root_event_id: root_event_id.to_owned(),
            },
        }
    }

    #[tokio::test]
    async fn authoritative_trust_runs_through_app_actor_ack_and_restarts_real_children() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let homeserver = format!("http://{}", listener.local_addr().expect("address"));
        std::thread::spawn(move || {
            for _ in 0..4096 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                std::thread::spawn(move || {
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
                    let body = if text.starts_with("GET /_matrix/client/versions ") {
                        r#"{"versions":["v1.7"],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#
                    } else if text.contains("/_matrix/client/") && text.contains("login") {
                        r#"{"access_token":"fixture-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#
                    } else if text
                        .contains("/_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
                    {
                        if text.contains("\"conn_id\":\"room-list\"") {
                            r#"{"pos":"sliding-pos","lists":{"all_rooms":{"count":1,"ops":[{"op":"SYNC","range":[0,0],"room_ids":["!fixture-room:example.invalid"]}]}},"rooms":{"!fixture-room:example.invalid":{"initial":true,"required_state":[{"type":"m.room.create","state_key":"","sender":"@fixture-user:example.invalid","event_id":"$create:example.invalid","origin_server_ts":1,"content":{"creator":"@fixture-user:example.invalid","room_version":"10"}},{"type":"m.room.name","state_key":"","sender":"@fixture-user:example.invalid","event_id":"$name:example.invalid","origin_server_ts":2,"content":{"name":"Fixture room"}},{"type":"m.room.member","state_key":"@fixture-user:example.invalid","sender":"@fixture-user:example.invalid","event_id":"$member:example.invalid","origin_server_ts":3,"content":{"membership":"join"}}]}},"extensions":{}}"#
                        } else {
                            r#"{"pos":"sliding-pos"}"#
                        }
                    } else if text.contains("/_matrix/client/") && text.contains("/sync") {
                        r#"{"next_batch":"batch","device_lists":{"changed":[],"left":[]},"rooms":{"invite":{},"join":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_one_time_keys_count":{}}"#
                    } else {
                        r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#
                    };
                    let body = if text
                        .contains("/_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
                    {
                        let mut response: serde_json::Value =
                            serde_json::from_str(body).expect("sliding-sync fixture response");
                        if let Some(txn_id) = text
                            .split_once("\r\n\r\n")
                            .and_then(|(_, body)| {
                                serde_json::from_str::<serde_json::Value>(body).ok()
                            })
                            .and_then(|request| request.get("txn_id").cloned())
                        {
                            response["txn_id"] = txn_id;
                        }
                        response.to_string()
                    } else {
                        body.to_owned()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).expect("write");
                });
            }
        });

        let data_dir = tempfile::tempdir().expect("data tempdir");
        let credential_dir = tempfile::tempdir().expect("credential tempdir");
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            runtime
                .account_actor_test_handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
        let (trust_tx, trust_rx) = mpsc::unbounded_channel();
        let updates = futures_util::stream::unfold(trust_rx, |mut rx| async move {
            rx.recv().await.map(|trust| (trust, rx))
        });
        assert!(
            runtime
                .account_actor_test_handle
                .send(AccountMessage::ConfigureTrustObservation {
                    observation: koushi_sdk::CurrentDeviceTrustObservation {
                        current: koushi_state::CurrentDeviceTrustState::Verified,
                        updates: Box::pin(updates),
                    },
                })
                .await
        );
        let connection = runtime.attach();
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id,
                request: koushi_state::LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: Some("Runtime Trust Test".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .expect("login command");

        wait_for_runtime_session(&runtime, "initial promotion", |session| {
            matches!(session, SessionState::Ready(_))
        })
        .await;
        wait_for_runtime_sync_running(&runtime, "initial promotion").await;
        assert_eq!(
            inspect_runtime_children(&runtime).await,
            (true, true, true, true)
        );
        assert_eq!(probe_rx.recv().await, Some("ready_projection_ack"));
        assert_eq!(
            probe_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty),
            "trust observer must remain active in Ready without another lifecycle token"
        );

        trust_tx
            .send(koushi_state::CurrentDeviceTrustState::Unverified)
            .expect("lock update");
        wait_for_runtime_session(&runtime, "trust revocation lock", |session| {
            matches!(session, SessionState::Locked(_))
        })
        .await;
        assert_eq!(
            inspect_runtime_children(&runtime).await,
            (true, false, false, true)
        );
        let mut tokens = Vec::new();
        while tokens.len() < 11 {
            tokens.push(probe_rx.recv().await.expect("stop token"));
        }
        assert_eq!(tokens[0], "lock_projection_ack");
        assert!(!tokens.contains(&"provisional_encryption_sync_terminated"));
        assert!(tokens.contains(&"stop_sync_actor"));
        assert!(tokens.contains(&"stop_timeline_manager"));
        assert!(tokens.contains(&"clear_room_session"));

        trust_tx
            .send(koushi_state::CurrentDeviceTrustState::Verified)
            .expect("repromotion update");
        wait_for_runtime_session(&runtime, "verified repromotion", |session| {
            matches!(session, SessionState::Ready(_))
        })
        .await;
        wait_for_runtime_sync_running(&runtime, "verified repromotion").await;
        assert_eq!(
            inspect_runtime_children(&runtime).await,
            (true, true, true, true)
        );
        assert_eq!(probe_rx.recv().await, Some("ready_projection_ack"));

        let before = runtime.snapshot_rx.borrow().state.session.clone();
        assert!(
            runtime
                .account_actor_test_handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 0,
                    trust: koushi_state::CurrentDeviceTrustState::Unverified,
                })
                .await
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            runtime.snapshot_rx.borrow().state.session,
            before,
            "stale/wrong-account trust changed state"
        );
        runtime.shutdown_handle().abort();
    }

    async fn wait_for_app_actor_shutdown(runtime: &CoreRuntime) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !runtime.shutdown_handle().is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("AppActor shutdown handle should complete");
    }

    #[tokio::test]
    async fn signed_out_shutdown_completes_app_actor_shutdown_handle() {
        let data_dir = tempfile::tempdir().expect("runtime data dir");
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let connection = runtime.attach();
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(AppCommand::Shutdown { request_id }))
            .await
            .expect("signed-out shutdown command");

        wait_for_app_actor_shutdown(&runtime).await;
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn first_shutdown_publishes_preceding_state_and_ignores_duplicate_and_later_commands() {
        let data_dir = tempfile::tempdir().expect("runtime data dir");
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let mut connection = runtime.attach();
        let first_request_id = connection.next_request_id();
        let shutdown_request_id = connection.next_request_id();
        let duplicate_shutdown_request_id = connection.next_request_id();
        let later_request_id = connection.next_request_id();

        runtime
            .command_tx
            .send(CoreCommandEnvelope {
                command: CoreCommand::App(AppCommand::UpdateSettings {
                    request_id: first_request_id,
                    patch: SettingsPatch {
                        thread_list_order: Some(koushi_state::ThreadListOrder::RootChronology),
                        ..SettingsPatch::default()
                    },
                }),
                composer_permit: None,
            })
            .await
            .expect("preceding command");
        runtime
            .command_tx
            .send(CoreCommandEnvelope {
                command: CoreCommand::App(AppCommand::Shutdown {
                    request_id: shutdown_request_id,
                }),
                composer_permit: None,
            })
            .await
            .expect("first shutdown command");
        runtime
            .command_tx
            .send(CoreCommandEnvelope {
                command: CoreCommand::App(AppCommand::Shutdown {
                    request_id: duplicate_shutdown_request_id,
                }),
                composer_permit: None,
            })
            .await
            .expect("duplicate shutdown command");
        runtime
            .command_tx
            .send(CoreCommandEnvelope {
                command: CoreCommand::App(AppCommand::UpdateSettings {
                    request_id: later_request_id,
                    patch: SettingsPatch {
                        room_list_sort: Some(koushi_state::RoomListSort::RecentFirst),
                        ..SettingsPatch::default()
                    },
                }),
                composer_permit: None,
            })
            .await
            .expect("later command");

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(
                    connection.recv_event().await,
                    Ok(CoreEvent::StateDelta(ref delta)) if delta.changed.settings.is_some()
                ) {
                    break;
                }
            }
        })
        .await
        .expect("preceding settings delta must publish before shutdown completes");
        wait_for_app_actor_shutdown(&runtime).await;
        let snapshot = runtime.snapshot_rx.borrow();
        assert_eq!(
            snapshot.state.settings.values.thread_list_order,
            koushi_state::ThreadListOrder::RootChronology
        );
        assert_eq!(
            snapshot.state.settings.values.room_list_sort,
            koushi_state::RoomListSort::Activity,
            "commands queued after the first Shutdown must not be handled"
        );
        drop(snapshot);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn explicit_shutdown_is_a_barrier_before_same_data_dir_reopen() {
        let data_dir = tempfile::tempdir().expect("runtime data dir");
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let connection = runtime.attach();
        drop(connection);
        tokio::time::timeout(Duration::from_secs(3), runtime.shutdown())
            .await
            .expect("first runtime shutdown barrier");

        let reopened = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let connection = reopened.attach();
        drop(connection);
        tokio::time::timeout(Duration::from_secs(3), reopened.shutdown())
            .await
            .expect("reopened runtime shutdown barrier");
    }

    async fn inspect_runtime_children(runtime: &CoreRuntime) -> (bool, bool, bool, bool) {
        let (response, result) = oneshot::channel();
        assert!(
            runtime
                .account_actor_test_handle
                .send(AccountMessage::InspectSessionRuntime { response })
                .await
        );
        result.await.expect("runtime inspection")
    }

    async fn wait_for_runtime_session(
        runtime: &CoreRuntime,
        stage: &'static str,
        predicate: impl Fn(&SessionState) -> bool,
    ) {
        let mut snapshot_rx = runtime.snapshot_rx.clone();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if predicate(&snapshot_rx.borrow().state.session) {
                    return;
                }
                snapshot_rx
                    .changed()
                    .await
                    .unwrap_or_else(|_| panic!("snapshot channel closed during {stage}"));
            }
        })
        .await
        .unwrap_or_else(|_| panic!("session transition timed out during {stage}"));
    }

    async fn wait_for_runtime_sync_running(runtime: &CoreRuntime, stage: &'static str) {
        let mut snapshot_rx = runtime.snapshot_rx.clone();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if matches!(
                    snapshot_rx.borrow().state.sync,
                    koushi_state::SyncState::Running
                ) {
                    return;
                }
                snapshot_rx
                    .changed()
                    .await
                    .unwrap_or_else(|_| panic!("snapshot channel closed during {stage}"));
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "sync start timed out during {stage}: {:?}",
                runtime.snapshot_rx.borrow().state.sync
            )
        });
    }
}
