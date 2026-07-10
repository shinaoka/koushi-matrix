//! `CoreRuntime`, `CoreConnection`, and the `AppActor` loop.
//!
//! Channel topology (overview.md, Async rule 10):
//! - command inbox per runtime: bounded mpsc, capacity 256
//! - discrete core events per consumer: broadcast, capacity 16384; a lagged
//!   consumer observes `EventStreamLag` and resyncs from the snapshot watch
//! - state snapshots: latest-wins watch, coalesced to at most one
//!   `StateDelta` per processed command batch

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    AccountManagementOperation, ActivityMarkReadTarget, ActivityRow, ActivityRowKind,
    ActivityState, ActivityStream, ActivityTab, AppAction, AppEffect, AppState, ComposerDraftStore,
    ProfileUpdateRequest, RoomNotificationMode, RoomSummary, ScheduledSendCapability,
    ScheduledSendHandle, ScheduledSendItem, SearchScope as AppSearchScope, SessionState,
    SpaceSummary, ThreadPaneState, UiEvent, reduce, room_activity_unread_count,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account::{AccountActorHandle, AccountMessage};
use crate::command::{
    AccountCommand, AppCommand, CoreCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand,
};
use crate::event::{
    ActivityEvent, AppStateSnapshot, CoreEvent, IntentNoOpReason, IntentOutcome, TimelineEvent,
    VersionedAppStateSnapshot, project_room_event_display_labels,
    project_timeline_event_display_labels,
};
use crate::executor;
use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use crate::settings::SettingsStore;
use crate::state_delta::build_state_delta;
use crate::store::{StoreActor, session_key_id_from_info};
use crate::unread_trace;

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
pub const COMPOSER_DRAFT_PERSIST_DEBOUNCE: Duration = Duration::from_millis(150);
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

fn reduce_with_unread_diagnostics(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    let room_list_trace = match &action {
        AppAction::RoomListUpdated { rooms, .. } => {
            Some(unread_trace::capture_room_list_applied(rooms))
        }
        _ => None,
    };
    let effects = reduce(state, action);
    if let Some(input) = room_list_trace {
        unread_trace::trace_room_list_applied(&input, &state.rooms);
    }
    effects
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CommandSubmitError {
    #[error("core runtime is closed")]
    RuntimeClosed,
    #[error("request id does not belong to this connection")]
    InvalidRequestId,
}

/// Surfaced when a consumer fell behind the bounded event queue. The
/// consumer must resync from the latest snapshot and (in later phases) the
/// per-timeline resync events; intermediate discrete events were dropped
/// for this consumer only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventStreamLag {
    pub skipped: u64,
}

/// Owns the actor tree and creates [`CoreConnection`] handles.
pub struct CoreRuntime {
    command_tx: mpsc::Sender<CoreCommand>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    next_connection_id: AtomicU64,
    // Internal action channel: actors project side-effect outcomes through
    // the reducer with this in later phases; tests inject through it today.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    action_tx: mpsc::Sender<Vec<AppAction>>,
    actor: executor::JoinHandle<()>,
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
        let account_store_actor = StoreActor::with_os_backend(data_dir.clone(), os_backend.clone());
        let composer_draft_store_actor = StoreActor::with_os_backend(data_dir.clone(), os_backend);
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

    #[cfg(any(test, feature = "test-hooks"))]
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
        let settings_store = SettingsStore::new(&data_dir);

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
        let account_actor = crate::account::AccountActor::spawn(
            store_actor,
            action_tx.clone(),
            event_tx.clone(),
            crate::link_preview::LinkPreviewContext::from_settings(&initial_state.settings.values),
        );

        let actor = AppActor {
            command_rx,
            action_rx,
            event_tx: event_tx.clone(),
            snapshot_tx,
            state: initial_state,
            settings_store,
            composer_draft_store_actor,
            composer_draft_loaded_for: None,
            navigation_loaded_for: None,
            scheduled_sends_loaded_for: None,
            room_preferences_loaded_for: None,
            state_generation: 0,
            pending_composer_draft_persist: None,
            account_actor,
            activity_projection: ActivityProjection::default(),
            next_internal_request_sequence: 1,
            pending_select: HashMap::new(),
        };
        let actor = executor::spawn(actor.run());

        Self {
            command_tx,
            event_tx,
            snapshot_rx,
            next_connection_id: AtomicU64::new(1),
            action_tx,
            actor,
        }
    }

    /// Attach a consumer. Returns its connection handle; the handle's
    /// `RuntimeConnectionId` is the only id its commands may carry.
    pub fn attach(&self) -> CoreConnection {
        CoreConnection {
            connection_id: RuntimeConnectionId(
                self.next_connection_id.fetch_add(1, Ordering::Relaxed),
            ),
            command_tx: self.command_tx.clone(),
            event_rx: self.event_tx.subscribe(),
            snapshot_rx: self.snapshot_rx.clone(),
            next_sequence: AtomicU64::new(1),
        }
    }

    /// Test hook: inject reducer actions as if an actor side effect produced
    /// them. Not part of the public production API.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn inject_actions(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.send(actions).await;
    }

    pub fn shutdown_handle(&self) -> &executor::JoinHandle<()> {
        &self.actor
    }
}

/// One attached consumer: allocates request ids, submits commands, and
/// observes the shared event stream plus the latest snapshot.
pub struct CoreConnection {
    connection_id: RuntimeConnectionId,
    command_tx: mpsc::Sender<CoreCommand>,
    event_rx: broadcast::Receiver<CoreEvent>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    next_sequence: AtomicU64,
}

/// Lightweight command submitter that can be cloned without cloning event or
/// snapshot receivers.
#[derive(Clone)]
pub struct CoreCommandHandle {
    connection_id: RuntimeConnectionId,
    command_tx: mpsc::Sender<CoreCommand>,
}

impl CoreCommandHandle {
    /// Submit a command. Fails locally — before routing and before any
    /// `CoreEvent` is published — if the command's request id does not
    /// belong to this connection.
    pub async fn command(&self, command: CoreCommand) -> Result<(), CommandSubmitError> {
        if command.request_id().connection_id != self.connection_id {
            return Err(CommandSubmitError::InvalidRequestId);
        }
        self.command_tx
            .send(command)
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }
}

impl CoreConnection {
    pub fn connection_id(&self) -> RuntimeConnectionId {
        self.connection_id
    }

    /// Clone a lightweight command submitter for callers that must not hold
    /// the full connection guard while awaiting bounded channel capacity.
    pub fn command_handle(&self) -> CoreCommandHandle {
        CoreCommandHandle {
            connection_id: self.connection_id,
            command_tx: self.command_tx.clone(),
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

    /// Submit a command. Fails locally — before routing and before any
    /// `CoreEvent` is published — if the command's request id does not
    /// belong to this connection.
    pub async fn command(&self, command: CoreCommand) -> Result<(), CommandSubmitError> {
        self.command_handle().command(command).await
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

    fn project_event_for_consumer(&self, mut event: CoreEvent) -> CoreEvent {
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
            | CoreEvent::StateChanged(_)
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
}

struct AppActor {
    command_rx: mpsc::Receiver<CoreCommand>,
    action_rx: mpsc::Receiver<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_tx: watch::Sender<VersionedAppStateSnapshot>,
    state: AppState,
    settings_store: SettingsStore,
    composer_draft_store_actor: StoreActor,
    composer_draft_loaded_for: Option<koushi_key::SessionKeyId>,
    navigation_loaded_for: Option<koushi_key::SessionKeyId>,
    scheduled_sends_loaded_for: Option<koushi_key::SessionKeyId>,
    room_preferences_loaded_for: Option<koushi_key::SessionKeyId>,
    state_generation: u64,
    pending_composer_draft_persist: Option<PendingComposerDraftPersist>,
    account_actor: AccountActorHandle,
    activity_projection: ActivityProjection,
    next_internal_request_sequence: u64,
    /// Correlation map for SelectRoom intents: room_id → FIFO queue of request_ids.
    /// Multiple concurrent SelectRoom commands for the same room are queued in
    /// submission order; each `AppAction::SelectRoom` pops the oldest entry so
    /// every submitted command receives a terminal `IntentLifecycle` outcome.
    /// Private-data-free: stores opaque ids only, never room names or content.
    pending_select: HashMap<String, std::collections::VecDeque<RequestId>>,
}

struct PendingComposerDraftPersist {
    key_id: koushi_key::SessionKeyId,
    drafts: ComposerDraftStore,
    deadline: Instant,
}

#[derive(Default)]
struct ActivityProjection {
    rows_by_event_id: BTreeMap<String, ActivityRow>,
    cleared_event_ids: BTreeSet<String>,
    /// Rooms whose placeholder unread row has just been cleared by a local
    /// mark-read. Suppresses re-synthesizing the placeholder until the reducer
    /// has had a chance to zero out the room's unread counts.
    cleared_placeholder_room_ids: BTreeSet<String>,
}

#[derive(Default)]
struct ActivityMarkReadResult {
    cleared_event_ids: Vec<String>,
    cleared_placeholder_room_ids: Vec<String>,
}

impl ActivityProjection {
    fn ingest(&mut self, rows: Vec<ActivityRow>) {
        for mut row in rows {
            if row.kind != ActivityRowKind::Event
                || row
                    .event_id
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                || row.room_id.trim().is_empty()
            {
                continue;
            }
            row.room_label.clear();
            row.unread = false;
            if let Some(event_id) = row.event_id.clone() {
                self.rows_by_event_id.insert(event_id, row);
            }
        }
    }

    fn mark_read(
        &mut self,
        state: &AppState,
        target: &ActivityMarkReadTarget,
    ) -> ActivityMarkReadResult {
        let (_recent, unread, _excluded) = self.snapshot(state);
        let mut cleared_event_ids = Vec::new();
        let mut cleared_placeholder_room_ids = Vec::new();
        let mut cleared_event_row_room_ids = BTreeSet::new();
        match target {
            ActivityMarkReadTarget::All => {
                for row in unread.rows {
                    match row.kind {
                        ActivityRowKind::Event => {
                            if let Some(event_id) = row.event_id {
                                cleared_event_ids.push(event_id);
                                cleared_event_row_room_ids.insert(row.room_id);
                            }
                        }
                        ActivityRowKind::RoomUnread => {
                            cleared_placeholder_room_ids.push(row.room_id);
                        }
                    }
                }
            }
            ActivityMarkReadTarget::Room {
                room_id,
                up_to_event_id,
            } => {
                let target_timestamp = unread
                    .rows
                    .iter()
                    .find(|row| {
                        row.room_id == *room_id
                            && row.event_id.as_deref() == Some(up_to_event_id.as_str())
                    })
                    .map(|row| row.timestamp_ms);
                for row in unread.rows {
                    if row.room_id != *room_id {
                        continue;
                    }
                    let matches_timestamp = target_timestamp
                        .map(|timestamp| row.timestamp_ms <= timestamp)
                        .unwrap_or(true);
                    if !matches_timestamp {
                        continue;
                    }
                    match row.kind {
                        ActivityRowKind::Event => {
                            if let Some(event_id) = row.event_id {
                                cleared_event_ids.push(event_id);
                                cleared_event_row_room_ids.insert(row.room_id);
                            }
                        }
                        ActivityRowKind::RoomUnread => {
                            cleared_placeholder_room_ids.push(row.room_id);
                        }
                    }
                }
            }
        }
        for event_id in &cleared_event_ids {
            self.cleared_event_ids.insert(event_id.clone());
        }
        for room_id in &cleared_placeholder_room_ids {
            self.cleared_placeholder_room_ids.insert(room_id.clone());
        }
        // Suppress placeholder synthesis for rooms whose event rows are being
        // cleared, until the reducer has zeroed out the room's unread counts.
        for room_id in cleared_event_row_room_ids {
            self.cleared_placeholder_room_ids.insert(room_id);
        }
        ActivityMarkReadResult {
            cleared_event_ids,
            cleared_placeholder_room_ids,
        }
    }

    fn fully_read_marker_updates(
        &mut self,
        state: &AppState,
        target: &ActivityMarkReadTarget,
    ) -> Vec<(String, String)> {
        match target {
            ActivityMarkReadTarget::Room {
                room_id,
                up_to_event_id,
            } => vec![(room_id.clone(), up_to_event_id.clone())],
            ActivityMarkReadTarget::All => {
                let (_recent, unread, _excluded) = self.snapshot(state);
                let rooms_by_id = state
                    .rooms
                    .iter()
                    .map(|room| (room.room_id.as_str(), room))
                    .collect::<HashMap<_, _>>();
                let mut latest_by_room: BTreeMap<String, (u64, String)> = BTreeMap::new();
                for row in unread.rows {
                    let event_id = match row.kind {
                        ActivityRowKind::Event => row.event_id,
                        ActivityRowKind::RoomUnread => rooms_by_id
                            .get(row.room_id.as_str())
                            .and_then(|room| room.latest_event.as_ref())
                            .map(|event| event.event_id.clone()),
                    };
                    if let Some(event_id) = event_id {
                        latest_by_room
                            .entry(row.room_id)
                            .and_modify(|(timestamp_ms, existing_event_id)| {
                                if row.timestamp_ms > *timestamp_ms {
                                    *timestamp_ms = row.timestamp_ms;
                                    *existing_event_id = event_id.clone();
                                }
                            })
                            .or_insert((row.timestamp_ms, event_id));
                    }
                }
                latest_by_room
                    .into_iter()
                    .map(|(room_id, (_timestamp_ms, event_id))| (room_id, event_id))
                    .collect()
            }
        }
    }

    fn event_at_or_after(&self, room_id: &str, timestamp_ms: u64) -> Option<String> {
        let mut rows = self
            .rows_by_event_id
            .values()
            .filter(|row| row.room_id == room_id)
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            left.timestamp_ms
                .cmp(&right.timestamp_ms)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });

        rows.iter()
            .find(|row| row.timestamp_ms >= timestamp_ms)
            .or_else(|| rows.last())
            .filter(|row| row.kind == ActivityRowKind::Event)
            .and_then(|row| row.event_id.clone())
    }

    fn update_action_for_open_state(&mut self, state: &AppState) -> Option<AppAction> {
        if !matches!(state.activity, ActivityState::Open { .. }) {
            return None;
        }
        let (recent, unread, excluded_room_ids) = self.snapshot(state);
        Some(AppAction::ActivityRowsUpdated {
            recent,
            unread,
            excluded_room_ids,
        })
    }

    fn room_ids_without_remaining_unread(
        &mut self,
        state: &AppState,
        cleared_event_ids: &[String],
    ) -> Vec<String> {
        let affected_room_ids = cleared_event_ids
            .iter()
            .filter_map(|event_id| self.rows_by_event_id.get(event_id))
            .map(|row| row.room_id.clone())
            .collect::<BTreeSet<_>>();
        if affected_room_ids.is_empty() {
            return Vec::new();
        }

        let (_recent, unread, _excluded_room_ids) = self.snapshot(state);
        let remaining_unread_room_ids = unread
            .rows
            .into_iter()
            .map(|row| row.room_id)
            .collect::<BTreeSet<_>>();
        affected_room_ids
            .into_iter()
            .filter(|room_id| !remaining_unread_room_ids.contains(room_id))
            .collect()
    }

    fn snapshot(&mut self, state: &AppState) -> (ActivityStream, ActivityStream, Vec<String>) {
        let rooms_by_id: HashMap<&str, &RoomSummary> = state
            .rooms
            .iter()
            .map(|room| (room.room_id.as_str(), room))
            .collect();
        let excluded_room_ids = state
            .rooms
            .iter()
            .filter(|room| {
                room.tags.low_priority.is_some()
                    || state
                        .room_notification_settings
                        .get(&room.room_id)
                        .is_some_and(|settings| settings.mode == RoomNotificationMode::Mute)
            })
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        let excluded: BTreeSet<&str> = excluded_room_ids.iter().map(String::as_str).collect();

        let mut recent = Vec::new();
        let mut unread = Vec::new();
        let mut recent_event_ids = BTreeSet::new();
        for row in self.rows_by_event_id.values() {
            if excluded.contains(row.room_id.as_str()) {
                continue;
            }
            let Some(room) = rooms_by_id.get(row.room_id.as_str()) else {
                continue;
            };
            let fully_read_event_id = state
                .live_signals
                .rooms
                .get(row.room_id.as_str())
                .and_then(|signals| signals.fully_read_event_id.as_deref());
            let mode = state
                .room_notification_settings
                .get(&room.room_id)
                .map(|settings| settings.mode);
            let room_activity_unread = room_has_activity_unread(room, mode);
            let unread_by_marker = room_activity_unread
                && match fully_read_event_id {
                    Some(event_id) => match row.event_id.as_deref() {
                        Some(row_event_id) if row_event_id == event_id => false,
                        Some(_) => self
                            .rows_by_event_id
                            .get(event_id)
                            .map(|fully_read_row| row.timestamp_ms > fully_read_row.timestamp_ms)
                            .unwrap_or(room_activity_unread),
                        None => false,
                    },
                    None => true,
                };
            let unread_row = unread_by_marker
                && !self
                    .cleared_event_ids
                    .contains(row.event_id.as_deref().unwrap_or(""));
            if !activity_recent_row_visible(mode, row.highlight, room_activity_unread) {
                continue;
            }
            let sender_avatar = row
                .sender_id
                .as_ref()
                .and_then(|user_id| state.profile.users.get(user_id))
                .and_then(|profile| profile.avatar.clone())
                .or_else(|| row.sender_avatar.clone());
            let context_label = activity_row_context_label(room, &state.spaces);
            let row = ActivityRow {
                room_label: room.display_label.clone(),
                sender_avatar,
                context_label,
                unread: unread_row,
                highlight: row.highlight || (unread_row && room.highlight_count > 0),
                ..row.clone()
            };
            if let Some(event_id) = row.event_id.clone() {
                recent_event_ids.insert(event_id);
            }
            recent.push(row);
        }

        for room in state.rooms.iter() {
            if excluded.contains(room.room_id.as_str()) {
                continue;
            }
            let Some(latest_event) = &room.latest_event else {
                continue;
            };
            if recent_event_ids.contains(&latest_event.event_id) {
                continue;
            }
            let fully_read_event_id = state
                .live_signals
                .rooms
                .get(room.room_id.as_str())
                .and_then(|signals| signals.fully_read_event_id.as_deref());
            let mode = state
                .room_notification_settings
                .get(&room.room_id)
                .map(|settings| settings.mode);
            let room_activity_unread = room_has_activity_unread(room, mode);
            let has_room_metrics = room.unread_count > 0 || room_activity_unread;
            let unread_row = room_activity_unread
                && fully_read_event_id != Some(latest_event.event_id.as_str())
                && !self.cleared_event_ids.contains(&latest_event.event_id);
            if has_room_metrics {
                let reason = if !room_activity_unread {
                    "plain_unread_only"
                } else if unread_row {
                    "unread"
                } else if fully_read_event_id == Some(latest_event.event_id.as_str()) {
                    "fully_read_latest"
                } else {
                    "cleared_latest"
                };
                unread_trace::trace_activity_room(
                    "activity_recent_event",
                    room,
                    unread_row,
                    reason,
                );
            }
            let latest_event_highlight = unread_row && room.highlight_count > 0;
            if !activity_recent_row_visible(mode, latest_event_highlight, room_activity_unread) {
                continue;
            }
            let context_label = activity_row_context_label(room, &state.spaces);
            let mut row = ActivityRow::event(
                room.room_id.clone(),
                latest_event.event_id.clone(),
                latest_event.sender_id.clone(),
                room.display_label.clone(),
                latest_event.sender_label.clone(),
                latest_event.preview.clone(),
                latest_event.timestamp_ms,
                unread_row,
                latest_event_highlight,
            );
            row.sender_avatar = latest_event.sender_avatar.clone();
            row.context_label = context_label;
            recent.push(row);
        }

        for room in state.rooms.iter() {
            if excluded.contains(room.room_id.as_str()) {
                continue;
            }
            let mode = state
                .room_notification_settings
                .get(&room.room_id)
                .map(|settings| settings.mode);
            let has_room_metrics = room.unread_count > 0 || room_has_activity_unread(room, mode);
            if !has_room_metrics {
                continue;
            }
            if !room_has_activity_unread(room, mode) {
                unread_trace::trace_activity_room(
                    "activity_placeholder",
                    room,
                    false,
                    "plain_unread_only",
                );
                continue;
            }
            if self.cleared_placeholder_room_ids.contains(&room.room_id) {
                unread_trace::trace_activity_room(
                    "activity_placeholder",
                    room,
                    false,
                    "cleared_local",
                );
                continue;
            }
            let highlight = room.highlight_count > 0;
            let timestamp_ms = room.last_activity_ms;
            let context_label = activity_row_context_label(room, &state.spaces);
            let placeholder = ActivityRow::room_unread_placeholder(
                room.room_id.clone(),
                room.display_label.clone(),
                timestamp_ms,
                highlight,
            );
            let placeholder = ActivityRow {
                context_label,
                ..placeholder
            };
            unread_trace::trace_activity_room("activity_placeholder", room, true, "room_metrics");
            unread.push(placeholder);
        }

        self.cleared_placeholder_room_ids.retain(|room_id| {
            rooms_by_id
                .get(room_id.as_str())
                .map(|room| {
                    let mode = state
                        .room_notification_settings
                        .get(&room.room_id)
                        .map(|settings| settings.mode);
                    room_has_activity_unread(room, mode)
                })
                .unwrap_or(false)
        });

        sort_activity_rows(&mut recent);
        sort_activity_rows(&mut unread);

        (
            ActivityStream {
                rows: recent,
                next_batch: None,
            },
            ActivityStream {
                rows: unread,
                next_batch: None,
            },
            excluded_room_ids,
        )
    }
}

fn room_has_activity_unread(room: &RoomSummary, mode: Option<RoomNotificationMode>) -> bool {
    room_activity_unread_count_for_mode(room, mode) > 0
}

fn room_activity_unread_count_for_mode(
    room: &RoomSummary,
    mode: Option<RoomNotificationMode>,
) -> u64 {
    if matches!(mode, Some(RoomNotificationMode::Mentions)) && room.highlight_count == 0 {
        0
    } else {
        room_activity_unread_count(room)
    }
}

fn activity_recent_row_visible(
    mode: Option<RoomNotificationMode>,
    row_highlight: bool,
    room_activity_unread: bool,
) -> bool {
    !matches!(mode, Some(RoomNotificationMode::Mentions)) || row_highlight || room_activity_unread
}

fn activity_row_context_label(room: &RoomSummary, spaces: &[SpaceSummary]) -> String {
    if room.is_dm {
        return "DM".to_owned();
    }
    let parent_space = room
        .parent_space_ids
        .iter()
        .filter_map(|space_id| spaces.iter().find(|space| space.space_id == *space_id))
        .next()
        .or_else(|| {
            spaces.iter().find(|space| {
                space
                    .child_room_ids
                    .iter()
                    .any(|room_id| room_id == &room.room_id)
            })
        });
    if let Some(space) = parent_space {
        return format!("{} / {}", space.display_name, room.display_label);
    }
    room.display_label.clone()
}

fn sort_activity_rows(rows: &mut [ActivityRow]) {
    rows.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.room_id.cmp(&right.room_id))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn scheduled_send_id() -> String {
    format!("scheduled-{}", matrix_sdk::ruma::TransactionId::new())
}

impl AppActor {
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
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break };
                    let loop_started = std::time::Instant::now();
                    let before_state = self.state.clone();
                    let clone_ms = loop_started.elapsed().as_millis();
                    let mut state_changed = self.handle_command(command).await;
                    let mut handled = 1u32;
                    // Coalesce: drain whatever is already queued before
                    // emitting a single StateChanged for the batch.
                    while let Ok(next) = self.command_rx.try_recv() {
                        state_changed |= self.handle_command(next).await;
                        handled += 1;
                    }
                    if state_changed {
                        self.publish_state_delta(&before_state);
                    }
                    app_loop_trace("command", handled, clone_ms, loop_started.elapsed());
                }
                actions = self.action_rx.recv() => {
                    let Some(actions) = actions else { break };
                    let loop_started = std::time::Instant::now();
                    let action_batch = actions.len() as u32;
                    let before_state = self.state.clone();
                    let clone_ms = loop_started.elapsed().as_millis();
                    let mut state_changed = false;
                    for action in actions {
                        if let AppAction::ActivityRowsObserved { rows } = &action {
                            self.activity_projection.ingest(rows.clone());
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
                                        | SessionState::NeedsRecovery { .. }
                                        | SessionState::Recovering { .. }
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
                        let cancel_replaced_room_timeline_pagination =
                            if let AppAction::SelectRoom { room_id } = &action {
                                self.cancel_replaced_room_timeline_pagination(room_id)
                            } else {
                                None
                            };
                        let cancel_replaced_room_timeline_link_previews =
                            if let AppAction::SelectRoom { room_id } = &action {
                                self.cancel_replaced_room_timeline_link_previews(room_id)
                            } else {
                                None
                            };
                        let post_projection_effects = self.reduce_app_action(action).await;
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
                            if committed {
                                if let Some(key) = cancel_replaced_room_timeline_pagination {
                                    let cancel_request_id = request_id_to_emit.unwrap_or(RequestId {
                                        connection_id: RuntimeConnectionId(0),
                                        sequence: 0,
                                    });
                                    self.send_timeline_command_or_fail(
                                        cancel_request_id,
                                        TimelineCommand::CancelPagination {
                                            request_id: cancel_request_id,
                                            key,
                                        },
                                    )
                                    .await;
                                }
                                if let Some(key) = cancel_replaced_room_timeline_link_previews {
                                    let cancel_request_id = request_id_to_emit.unwrap_or(RequestId {
                                        connection_id: RuntimeConnectionId(0),
                                        sequence: 0,
                                    });
                                    self.send_timeline_command_or_fail(
                                        cancel_request_id,
                                        TimelineCommand::CancelLinkPreviews {
                                            request_id: cancel_request_id,
                                            key,
                                        },
                                    )
                                    .await;
                                }
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
                        }
                        if let Some(activity_update) = self
                            .activity_projection
                            .update_action_for_open_state(&self.state)
                        {
                            let _activity_effects =
                                self.reduce_app_action(activity_update).await;
                        }
                        self.handle_post_projection_effects(&post_projection_effects)
                            .await;
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

    async fn reduce_app_action(&mut self, action: AppAction) -> Vec<AppEffect> {
        let previous_session = composer_draft_session_key(&self.state);
        let previous_drafts = self.state.composer_drafts.clone();
        let previous_navigation_session = navigation_session_key(&self.state);
        let previous_navigation = self.state.navigation.clone();
        let previous_scheduled_session = scheduled_send_session_key(&self.state);
        let previous_scheduled_sends = self.state.scheduled_sends.clone();
        let effects = reduce_with_unread_diagnostics(&mut self.state, action);
        if previous_navigation != self.state.navigation {
            let target_session =
                navigation_session_key(&self.state).or(previous_navigation_session);
            if let Some(key_id) = target_session {
                self.persist_navigation(key_id).await;
            }
        }
        if previous_drafts != self.state.composer_drafts {
            let target_session = composer_draft_session_key(&self.state).or(previous_session);
            if let Some(key_id) = target_session {
                self.schedule_composer_draft_persist(key_id, self.state.composer_drafts.clone())
                    .await;
            }
        }
        if previous_scheduled_sends != self.state.scheduled_sends {
            let current_scheduled_session = scheduled_send_session_key(&self.state);
            let cleared_for_session_transition = self.state.scheduled_sends.items.is_empty()
                && previous_scheduled_session.is_some()
                && current_scheduled_session.is_none();

            if cleared_for_session_transition {
                // `clear_session_views` intentionally clears the in-memory
                // projection on lock, logout, and account switch. That is not
                // a user cancellation, so do not overwrite the account's
                // persisted scheduled sends with an empty store.
                self.scheduled_sends_loaded_for = None;
            } else if let Some(key_id) = current_scheduled_session.or(previous_scheduled_session) {
                self.persist_scheduled_sends(key_id).await;
            }
        }
        effects
    }

    async fn load_navigation_for_current_session(&mut self) {
        let Some(key_id) = navigation_session_key(&self.state) else {
            self.navigation_loaded_for = None;
            return;
        };
        if self.navigation_loaded_for.as_ref() == Some(&key_id) {
            return;
        }

        let store = self.composer_draft_store_actor.clone();
        let load_key_id = key_id.clone();
        let navigation = executor::spawn_blocking(move || {
            store.load_navigation(&load_key_id).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let effects = reduce(&mut self.state, AppAction::NavigationLoaded { navigation });
        self.navigation_loaded_for = Some(key_id);
        self.handle_ui_event_effects(&effects).await;
    }

    async fn load_composer_drafts_for_current_session(&mut self) {
        let Some(key_id) = composer_draft_session_key(&self.state) else {
            self.composer_draft_loaded_for = None;
            return;
        };
        if self.composer_draft_loaded_for.as_ref() == Some(&key_id) {
            return;
        }

        let store = self.composer_draft_store_actor.clone();
        let load_key_id = key_id.clone();
        let drafts = executor::spawn_blocking(move || {
            store.load_composer_drafts(&load_key_id).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let effects = reduce(&mut self.state, AppAction::ComposerDraftsLoaded { drafts });
        self.composer_draft_loaded_for = Some(key_id);
        self.handle_ui_event_effects(&effects).await;
    }

    async fn load_scheduled_sends_for_current_session(&mut self) {
        let Some(key_id) = scheduled_send_session_key(&self.state) else {
            self.scheduled_sends_loaded_for = None;
            return;
        };
        if self.scheduled_sends_loaded_for.as_ref() == Some(&key_id) {
            return;
        }

        let store = self.composer_draft_store_actor.clone();
        let load_key_id = key_id.clone();
        let scheduled_sends = executor::spawn_blocking(move || {
            store.load_scheduled_sends(&load_key_id).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let effects = reduce(
            &mut self.state,
            AppAction::ScheduledSendsLoaded { scheduled_sends },
        );
        self.scheduled_sends_loaded_for = Some(key_id);
        self.handle_ui_event_effects(&effects).await;
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

    async fn persist_scheduled_sends(&mut self, key_id: koushi_key::SessionKeyId) {
        let store = self.composer_draft_store_actor.clone();
        let scheduled_sends = self.state.scheduled_sends.clone();
        let _ =
            executor::spawn_blocking(move || store.save_scheduled_sends(&key_id, &scheduled_sends))
                .await;
    }

    async fn persist_navigation(&mut self, key_id: koushi_key::SessionKeyId) {
        let store = self.composer_draft_store_actor.clone();
        let navigation = self.state.navigation.clone();
        let _ = executor::spawn_blocking(move || store.save_navigation(&key_id, &navigation)).await;
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

    async fn schedule_composer_draft_persist(
        &mut self,
        key_id: koushi_key::SessionKeyId,
        drafts: ComposerDraftStore,
    ) {
        if self
            .pending_composer_draft_persist
            .as_ref()
            .is_some_and(|pending| pending.key_id != key_id)
        {
            self.flush_pending_composer_drafts().await;
        }
        self.pending_composer_draft_persist = Some(PendingComposerDraftPersist {
            key_id,
            drafts,
            deadline: Instant::now() + COMPOSER_DRAFT_PERSIST_DEBOUNCE,
        });
    }

    fn composer_draft_persist_delay(&self) -> Option<Duration> {
        self.pending_composer_draft_persist
            .as_ref()
            .map(|pending| pending.deadline.saturating_duration_since(Instant::now()))
    }

    async fn flush_pending_composer_drafts(&mut self) {
        let Some(pending) = self.pending_composer_draft_persist.take() else {
            return;
        };
        let store = self.composer_draft_store_actor.clone();
        let _ = executor::spawn_blocking(move || {
            store.save_composer_drafts(&pending.key_id, &pending.drafts)
        })
        .await;
    }

    fn scheduled_send_delay(&self) -> Option<Duration> {
        if !matches!(self.state.session, SessionState::Ready(_)) {
            return None;
        }
        let next_send_at_ms = self.state.scheduled_sends.next_local_send_at_ms()?;
        let now_ms = current_epoch_ms();
        Some(Duration::from_millis(
            next_send_at_ms.saturating_sub(now_ms),
        ))
    }

    async fn dispatch_due_scheduled_send(&mut self) -> bool {
        if !matches!(self.state.session, SessionState::Ready(_)) {
            return false;
        }
        let Some(item) = self
            .state
            .scheduled_sends
            .next_local_due(current_epoch_ms())
        else {
            return false;
        };
        self.dispatch_scheduled_send(item).await
    }

    async fn dispatch_scheduled_send(&mut self, item: ScheduledSendItem) -> bool {
        let Some(origin_session_key) = scheduled_send_session_key(&self.state) else {
            return false;
        };
        let scheduled_id = item.scheduled_id.clone();
        let effects = self
            .reduce_app_action(AppAction::ScheduledSendDispatchStarted {
                scheduled_id: scheduled_id.clone(),
            })
            .await;
        self.handle_ui_event_effects(&effects).await;

        let request_id = self.next_internal_request_id();
        if !self
            .account_actor
            .send(AccountMessage::DispatchLocalScheduledSend {
                request_id,
                origin_session_key,
                scheduled_id: scheduled_id.clone(),
                room_id: item.room_id,
                body: item.body,
            })
            .await
        {
            self.emit(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::ShutdownFailed,
            });
            let retry_effects = self
                .reduce_app_action(AppAction::ScheduledSendDispatchFailed {
                    scheduled_id,
                    retry_at_ms: crate::scheduled_send::local_scheduled_send_retry_at_ms(),
                })
                .await;
            self.handle_ui_event_effects(&retry_effects).await;
        }
        true
    }

    fn next_internal_request_id(&mut self) -> RequestId {
        let sequence = self.next_internal_request_sequence;
        self.next_internal_request_sequence = self.next_internal_request_sequence.saturating_add(1);
        RequestId {
            connection_id: INTERNAL_RUNTIME_CONNECTION_ID,
            sequence,
        }
    }

    /// Returns whether `AppState` changed.
    async fn handle_command(&mut self, command: CoreCommand) -> bool {
        if command.requires_ready_session() && !is_ready_session_for_commands(&self.state.session) {
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
                request_id: command.request_id(),
                failure: CoreFailure::SessionRequired,
            });
            return false;
        }

        match command {
            CoreCommand::Account(account_command) => {
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
                self.handle_ui_event_effects_with_display_label_users(
                    &effects,
                    &display_label_user_ids,
                )
                .await;
                let should_route =
                    !matches!(&account_command, AccountCommand::ResetLocalData { .. })
                        || projected_state_changed;
                if !should_route {
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
                AppCommand::Shutdown { .. } => false,
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
                    room_id,
                    draft,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::ComposerDraftChanged { room_id, draft })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetThreadComposerDraft {
                    request_id,
                    room_id,
                    root_event_id,
                    draft,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::ThreadComposerDraftChanged {
                            room_id,
                            root_event_id,
                            draft,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetUploadStaging {
                    request_id,
                    room_id,
                    items,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingChanged { room_id, items })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::UpdateStagedUploadCaption {
                    request_id,
                    staged_id,
                    caption,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingCaptionChanged {
                            staged_id,
                            caption,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::UpdateStagedUploadCompression {
                    request_id,
                    staged_id,
                    compression_choice,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingCompressionChanged {
                            staged_id,
                            compression_choice,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::ClearUploadStaging {
                    request_id,
                    room_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::UploadStagingCleared { room_id })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::ScheduleSend {
                    request_id,
                    room_id,
                    body,
                    send_at_ms,
                } => {
                    if self.state.scheduled_sends.capability
                        != ScheduledSendCapability::LocalFallback
                    {
                        let scheduled_id = scheduled_send_id();
                        if !self
                            .account_actor
                            .send(AccountMessage::ScheduleServerDelayedSend {
                                request_id,
                                scheduled_id,
                                room_id,
                                body,
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
                        body,
                        send_at_ms,
                        handle: ScheduledSendHandle::Local,
                        is_dispatching: false,
                    };
                    let effects = self
                        .reduce_app_action(AppAction::ScheduledSendCreated { item })
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
                    send_at_ms,
                } => {
                    if let Some(item) = self.state.scheduled_sends.items.get(&scheduled_id).cloned()
                        && let ScheduledSendHandle::Server { delay_id } = item.handle
                    {
                        if !self
                            .account_actor
                            .send(AccountMessage::RescheduleServerDelayedSend {
                                request_id,
                                scheduled_id,
                                room_id: item.room_id,
                                body: item.body,
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
                } => {
                    let replaced_thread_key =
                        self.unsubscribe_replaced_thread_timeline(&room_id, &root_event_id);
                    let effects = self
                        .reduce_app_action(AppAction::OpenThread {
                            room_id,
                            root_event_id,
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
                AppCommand::ResetRoomTimelineCache {
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
                    let resubscribe =
                        self.state.navigation.active_room_id.as_deref() == Some(room_id.as_str());
                    let _ = self
                        .account_actor
                        .send(AccountMessage::ResetRoomTimelineCache {
                            request_id,
                            account_key,
                            room_id,
                            resubscribe,
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
                        let effects = self
                            .reduce_app_action(AppAction::OpenFocusedContext {
                                room_id: room_id.clone(),
                                event_id: event_id.clone(),
                            })
                            .await;
                        self.handle_app_effects(request_id, effects).await;
                        let anchor_effects = self
                            .reduce_app_action(AppAction::EnterAnchoredTimeline {
                                room_id,
                                event_id,
                            })
                            .await;
                        self.handle_app_effects(request_id, anchor_effects).await;
                        return true;
                    }
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
                    let effects = self
                        .reduce_app_action(AppAction::ActivityOpened {
                            request_id: request_id.sequence,
                        })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
                    let (recent, unread, excluded_room_ids) =
                        self.activity_projection.snapshot(&self.state);
                    let snapshot_effects = self
                        .reduce_app_action(AppAction::ActivitySnapshotLoaded {
                            request_id: request_id.sequence,
                            active_tab: ActivityTab::Recent,
                            recent: recent.clone(),
                            unread: unread.clone(),
                            excluded_room_ids,
                        })
                        .await;
                    self.handle_app_effects(request_id, snapshot_effects).await;
                    self.emit(CoreEvent::Activity(ActivityEvent::Opened { request_id }));
                    self.emit(CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                        request_id,
                        active_tab: ActivityTab::Recent,
                        recent,
                        unread,
                    }));
                    true
                }
                AppCommand::CloseActivity { request_id } => {
                    let effects = self.reduce_app_action(AppAction::ActivityClosed).await;
                    self.handle_app_effects(request_id, effects).await;
                    self.emit(CoreEvent::Activity(ActivityEvent::Closed { request_id }));
                    true
                }
                AppCommand::SetActivityTab { request_id, tab } => {
                    let effects = self
                        .reduce_app_action(AppAction::ActivityTabSelected { tab })
                        .await;
                    self.handle_app_effects(request_id, effects).await;
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
                AppCommand::OpenThreadsList {
                    request_id,
                    room_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::OpenThreadsList {
                            request_id: request_id.sequence,
                            room_id,
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
                AppCommand::PaginateThreadsList {
                    request_id,
                    room_id,
                } => {
                    let effects = self
                        .reduce_app_action(AppAction::PaginateThreadsList {
                            request_id: request_id.sequence,
                            room_id,
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
                // Route to AccountActor (which forwards to RoomActor).
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::RoomCommand(room_command))
                    .await;
                false
            }
            CoreCommand::Timeline(timeline_command) => {
                if self.should_suppress_timeline_command_for_privacy(&timeline_command) {
                    return false;
                }
                // Route to AccountActor (which forwards to TimelineManagerActor).
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::TimelineCommand(
                        timeline_command,
                    ))
                    .await;
                false
            }
            CoreCommand::Search(search_command) => {
                match search_command {
                    SearchCommand::Query {
                        request_id,
                        query,
                        scope,
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
                                room_id,
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
                                room_id,
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
                AppEffect::RestoreSession
                | AppEffect::DiscoverLogin { .. }
                | AppEffect::Login(_)
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
                | AppEffect::SendText { .. } => {}
            }
        }
    }

    async fn handle_post_projection_effects(&mut self, effects: &[AppEffect]) {
        for effect in effects {
            match effect {
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
                    let request_id = self.next_internal_request_id();
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
                                kind: TimelineKind::Room {
                                    room_id: room_id.clone(),
                                },
                            },
                            replay_existing: true,
                        },
                    )
                    .await;
                }
                AppEffect::PersistRoomPreferences { preferences, .. } => {
                    self.persist_room_preferences(preferences).await;
                }
                AppEffect::RestoreSession
                | AppEffect::DiscoverLogin { .. }
                | AppEffect::Login(_)
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
                | AppEffect::PaginateThreadsList { .. }
                | AppEffect::UnsubscribeThreadsList
                | AppEffect::NotifySearchCrawlerRoomsAvailable { .. }
                | AppEffect::InvalidateSearchCrawlerCache
                | AppEffect::RebuildSearchIndex
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
            SessionState::NeedsRecovery { info, .. }
            | SessionState::Recovering { info, .. }
            | SessionState::Ready(info)
            | SessionState::Locked(info) => Some(AccountKey(info.user_id.clone())),
            SessionState::SignedOut
            | SessionState::Restoring
            | SessionState::SwitchingAccount { .. }
            | SessionState::Authenticating { .. }
            | SessionState::LoggingOut => None,
        }
    }

    fn current_thread_timeline_key(&self) -> Option<TimelineKey> {
        let account_key = self.current_account_key()?;
        match &self.state.thread {
            ThreadPaneState::Opening {
                room_id,
                root_event_id,
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

    fn cancel_replaced_room_timeline_pagination(&self, room_id: &str) -> Option<TimelineKey> {
        cancel_replaced_room_timeline_pagination_key(self.current_room_timeline_key(), room_id)
    }

    fn cancel_replaced_room_timeline_link_previews(&self, room_id: &str) -> Option<TimelineKey> {
        cancel_replaced_room_timeline_link_previews_key(self.current_room_timeline_key(), room_id)
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

    fn current_focused_context_timeline_key(&self) -> Option<TimelineKey> {
        let account_key = self.current_account_key()?;
        match &self.state.focused_context {
            koushi_state::FocusedContextState::Opening { room_id, event_id }
            | koushi_state::FocusedContextState::Open {
                room_id, event_id, ..
            } => Some(TimelineKey {
                account_key,
                kind: TimelineKind::Focused {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                },
            }),
            koushi_state::FocusedContextState::Closed => None,
        }
    }

    fn unsubscribe_replaced_focused_context_timeline(
        &self,
        room_id: &str,
        event_id: &str,
    ) -> Option<TimelineKey> {
        let replacement_key = TimelineKey {
            account_key: self.current_account_key()?,
            kind: TimelineKind::Focused {
                room_id: room_id.to_owned(),
                event_id: event_id.to_owned(),
            },
        };
        unsubscribe_replaced_focused_context_timeline_key(
            self.current_focused_context_timeline_key(),
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

fn unsubscribe_replaced_focused_context_timeline_key(
    current_key: Option<TimelineKey>,
    replacement_key: TimelineKey,
) -> Option<TimelineKey> {
    unsubscribe_replaced_timeline_key(current_key, replacement_key)
}

fn unsubscribe_replaced_timeline_key(
    current_key: Option<TimelineKey>,
    replacement_key: TimelineKey,
) -> Option<TimelineKey> {
    current_key.filter(|current_key| current_key != &replacement_key)
}

fn cancel_replaced_room_timeline_pagination_key(
    current_key: Option<TimelineKey>,
    replacement_room_id: &str,
) -> Option<TimelineKey> {
    current_key.filter(|current_key| match &current_key.kind {
        TimelineKind::Room { room_id } => room_id != replacement_room_id,
        TimelineKind::Thread { .. } | TimelineKind::Focused { .. } => false,
    })
}

fn cancel_replaced_room_timeline_link_previews_key(
    current_key: Option<TimelineKey>,
    replacement_room_id: &str,
) -> Option<TimelineKey> {
    current_key.filter(|current_key| match &current_key.kind {
        TimelineKind::Room { room_id } => room_id != replacement_room_id,
        TimelineKind::Thread { .. } | TimelineKind::Focused { .. } => false,
    })
}

fn is_ready_session_for_commands(session: &SessionState) -> bool {
    matches!(
        session,
        SessionState::Ready(_)
            | SessionState::NeedsRecovery { .. }
            | SessionState::Recovering { .. }
    )
}

fn composer_draft_session_key(state: &AppState) -> Option<koushi_key::SessionKeyId> {
    match &state.session {
        SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. }
        | SessionState::Ready(info) => Some(session_key_id_from_info(info)),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut
        | SessionState::Locked(_) => None,
    }
}

fn navigation_session_key(state: &AppState) -> Option<koushi_key::SessionKeyId> {
    composer_draft_session_key(state)
}

fn scheduled_send_session_key(state: &AppState) -> Option<koushi_key::SessionKeyId> {
    composer_draft_session_key(state)
}

fn room_preferences_session_key(state: &AppState) -> Option<koushi_key::SessionKeyId> {
    composer_draft_session_key(state)
}

fn effects_open_thread_timeline(effects: &[AppEffect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, AppEffect::OpenThreadTimeline { .. }))
}

fn effects_open_focused_timeline(effects: &[AppEffect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, AppEffect::OpenFocusedTimeline { .. }))
}

fn map_core_search_scope_to_state(scope: SearchScope) -> AppSearchScope {
    match scope {
        SearchScope::AllRooms => AppSearchScope::AllRooms,
        SearchScope::CurrentRoom { room_id } => AppSearchScope::CurrentRoom { room_id },
        SearchScope::CurrentSpace { space_id } => AppSearchScope::CurrentSpace { space_id },
        SearchScope::Dms => AppSearchScope::Dms,
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
        AccountCommand::CancelVerification {
            flow_id, reason, ..
        } => Some(AppAction::VerificationCancelled {
            request_id: *flow_id,
            reason: *reason,
        }),
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
        AccountCommand::LoginPassword { .. }
        | AccountCommand::CompleteOidcLogin { .. }
        | AccountCommand::RestoreSession { .. }
        | AccountCommand::RestoreLastSession { .. }
        | AccountCommand::QuerySavedSessions { .. }
        | AccountCommand::SubmitRecovery { .. }
        | AccountCommand::SetPresence { .. }
        | AccountCommand::DownloadAvatarThumbnail { .. }
        | AccountCommand::Logout { .. }
        | AccountCommand::SwitchAccount { .. } => None,
    }
}

fn map_state_search_scope_to_core(scope: AppSearchScope) -> SearchScope {
    match scope {
        AppSearchScope::AllRooms => SearchScope::AllRooms,
        AppSearchScope::CurrentSpace { space_id } => SearchScope::CurrentSpace { space_id },
        AppSearchScope::Dms => SearchScope::Dms,
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
    use crate::event::{
        ThreadSummaryDto, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId,
    };
    use koushi_state::{
        DisplaySettings, LocalUserAliasUpdateState, OwnProfile, ProfileState,
        RoomLatestEventSummary, RoomNotificationModeOperation, RoomNotificationSettings,
        RoomSummary, RoomTags, SessionInfo, SettingsPatch, UserProfile, reduce,
    };

    fn unread_diagnostic_room(room_id: &str) -> RoomSummary {
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
            last_activity_ms: 42,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }
    }

    #[test]
    fn room_list_applied_records_through_real_reducer_with_trace_env_unset() {
        let child = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "runtime::tests::room_list_applied_records_without_trace_environment",
            "--ignored",
            "--nocapture",
        ])
        .env_remove("KOUSHI_UNREAD_TRACE")
        .status()
        .expect("env-unset room-list diagnostic child should start");
        assert!(
            child.success(),
            "env-unset diagnostic child failed: {child}"
        );
    }

    #[test]
    #[ignore]
    fn room_list_applied_records_without_trace_environment() {
        assert!(std::env::var_os("KOUSHI_UNREAD_TRACE").is_none());
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@synthetic:example.invalid".to_owned(),
                device_id: "SYNTHETIC".to_owned(),
            }),
            ..AppState::default()
        };
        let private_room_id = "!private-room:example.invalid";

        reduce_with_unread_diagnostics(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![unread_diagnostic_room(private_room_id)],
            },
        );

        assert_eq!(state.rooms.len(), 1, "the real reducer path should run");
        let event = koushi_diagnostics::snapshot()
            .records
            .into_iter()
            .rev()
            .find(|record| {
                record.event.source == "core.unread" && record.event.stage == "room_list_applied"
            })
            .expect("room-list applied metrics should be collected without an env switch")
            .event;
        assert_eq!(
            event
                .fields
                .iter()
                .map(|field| (field.key, field.value.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("unread", koushi_diagnostics::DiagnosticValue::Count(3)),
                (
                    "notifications",
                    koushi_diagnostics::DiagnosticValue::Count(2),
                ),
                ("highlights", koushi_diagnostics::DiagnosticValue::Count(1)),
                (
                    "marked_unread",
                    koushi_diagnostics::DiagnosticValue::Boolean(true),
                ),
                (
                    "latest_event_present",
                    koushi_diagnostics::DiagnosticValue::Boolean(false),
                ),
            ]
        );
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains(private_room_id)
        );
    }

    #[test]
    fn app_loop_trace_ignores_subthreshold_iterations() {
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
            to_state.contains("SearchScope::CurrentSpace")
                && to_state.contains("SearchScope::Dms")
                && to_state.contains("SearchScope::AllRooms"),
            "core search scopes must preserve current-space, DM, and all-rooms kinds in AppState"
        );
        assert!(
            to_core.contains("AppSearchScope::CurrentSpace")
                && to_core.contains("AppSearchScope::Dms")
                && to_core.contains("AppSearchScope::AllRooms"),
            "submitted AppState search scopes must round-trip through core without collapsing to global"
        );
    }

    #[test]
    fn core_connection_command_handle_clones_submit_path() {
        let source = include_str!("runtime.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("runtime production source should precede tests");
        let handle_impl = production_source
            .split("impl CoreCommandHandle")
            .nth(1)
            .expect("CoreConnection should expose a lightweight command handle");
        let connection_impl = production_source
            .split("impl CoreConnection")
            .nth(1)
            .expect("CoreConnection impl should exist");
        let command_handle_fn = connection_impl
            .split("pub fn command_handle")
            .nth(1)
            .expect("CoreConnection should clone a command handle for submitters")
            .split("pub fn next_request_id")
            .next()
            .expect("command_handle should precede request-id allocation");
        let command_fn = connection_impl
            .split("pub async fn command")
            .nth(1)
            .expect("CoreConnection command helper should exist")
            .split("pub async fn recv_event")
            .next()
            .expect("command helper should precede event receiving");

        assert!(
            production_source.contains("#[derive(Clone)]\npub struct CoreCommandHandle"),
            "the command submit path must be cloneable without cloning event/snapshot receivers"
        );
        assert!(
            handle_impl.contains("self.command_tx")
                && handle_impl.contains(".send(command)")
                && handle_impl.contains(".await"),
            "the command handle must own the bounded send await"
        );
        assert!(
            command_handle_fn.contains("command_tx: self.command_tx.clone()"),
            "CoreConnection::command_handle must clone only the bounded sender"
        );
        assert!(
            command_fn.contains("self.command_handle().command(command).await"),
            "CoreConnection::command should delegate through the same submit handle"
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

    #[test]
    fn activity_projection_ignores_plain_unread_count_for_activity_unread() {
        let mut state = AppState::default();
        state.rooms = vec![RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 42,
            latest_event: Some(RoomLatestEventSummary {
                event_id: "$latest:example.invalid".to_owned(),
                sender_id: Some("@sender:example.invalid".to_owned()),
                sender_label: Some("Sender".to_owned()),
                sender_avatar: None,
                preview: Some("body".to_owned()),
                timestamp_ms: 42,
            }),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }];

        let mut projection = ActivityProjection::default();
        let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

        assert!(
            unread.rows.is_empty(),
            "Activity Unread should not invent un-navigable rows from plain unread message counts"
        );
        assert_eq!(recent.rows.len(), 1);
        assert!(
            !recent.rows[0].unread,
            "plain unread message counts should not mark Activity recent rows unread"
        );
    }

    #[test]
    fn activity_projection_ignores_plain_unread_count_for_ingested_event_rows() {
        let mut state = AppState::default();
        state.rooms = vec![RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 42,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }];

        let mut projection = ActivityProjection::default();
        projection.ingest(vec![ActivityRow::event(
            "!room:example.invalid".to_owned(),
            "$event:example.invalid".to_owned(),
            Some("@sender:example.invalid".to_owned()),
            "Room".to_owned(),
            Some("Sender".to_owned()),
            Some("body".to_owned()),
            42,
            true,
            false,
        )]);
        let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

        assert!(unread.rows.is_empty());
        assert_eq!(recent.rows.len(), 1);
        assert!(
            !recent.rows[0].unread,
            "ingested event rows must not inherit plain unread-only state"
        );
    }

    #[test]
    fn activity_projection_skips_recent_rows_for_mentions_mode_without_highlight() {
        let mut state = AppState::default();
        state.rooms = vec![RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 1,
            notification_count: 1,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 42,
            latest_event: Some(RoomLatestEventSummary {
                event_id: "$latest:example.invalid".to_owned(),
                sender_id: Some("@sender:example.invalid".to_owned()),
                sender_label: Some("Sender".to_owned()),
                sender_avatar: None,
                preview: Some("body".to_owned()),
                timestamp_ms: 42,
            }),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }];
        state.room_notification_settings.insert(
            "!room:example.invalid".to_owned(),
            RoomNotificationSettings {
                mode: RoomNotificationMode::Mentions,
                operation: RoomNotificationModeOperation::Idle,
            },
        );

        let mut projection = ActivityProjection::default();
        projection.ingest(vec![ActivityRow::event(
            "!room:example.invalid".to_owned(),
            "$event:example.invalid".to_owned(),
            Some("@sender:example.invalid".to_owned()),
            "Room".to_owned(),
            Some("Sender".to_owned()),
            Some("body".to_owned()),
            41,
            true,
            false,
        )]);
        let (recent, unread, _excluded_room_ids) = projection.snapshot(&state);

        assert!(recent.rows.is_empty());
        assert!(unread.rows.is_empty());
    }

    #[test]
    fn activity_projection_context_label_uses_space_and_room_names() {
        let mut state = AppState::default();
        state.spaces = vec![SpaceSummary {
            space_id: "!space:example.invalid".to_owned(),
            display_name: "Science".to_owned(),
            avatar: None,
            child_room_ids: vec!["!room:example.invalid".to_owned()],
        }];
        state.rooms = vec![RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Papers".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 42,
            latest_event: Some(RoomLatestEventSummary {
                event_id: "$latest:example.invalid".to_owned(),
                sender_id: Some("@sender:example.invalid".to_owned()),
                sender_label: Some("Sender".to_owned()),
                sender_avatar: None,
                preview: Some("body".to_owned()),
                timestamp_ms: 42,
            }),
            parent_space_ids: vec!["!space:example.invalid".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }];

        let mut projection = ActivityProjection::default();
        let (recent, _unread, _excluded_room_ids) = projection.snapshot(&state);

        assert_eq!(recent.rows[0].context_label, "Science / Papers");
    }

    #[tokio::test]
    async fn versioned_snapshot_generation_matches_state_delta_generation() {
        let runtime = CoreRuntime::start_with_event_capacity(8);
        let mut connection = runtime.attach();

        runtime
            .inject_actions(vec![AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
            })])
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
    async fn connection_projects_timeline_sender_labels_from_latest_snapshot() {
        let (command_tx, _command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = broadcast::channel(4);
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
            }),
        );
        state.profile = ProfileState {
            own: OwnProfile {
                display_name: Some("Me Upstream".to_owned()),
                avatar: None,
            },
            ignored_user_ids: BTreeSet::new(),
            ignored_user_update: koushi_state::IgnoredUserUpdateState::Idle,
            users: BTreeMap::from([
                (
                    "@alice:example.invalid".to_owned(),
                    UserProfile {
                        user_id: "@alice:example.invalid".to_owned(),
                        display_name: Some("Alice Upstream".to_owned()),
                        display_label: "Alice Alias".to_owned(),
                        original_display_label: "Alice Upstream".to_owned(),
                        mention_search_terms: vec![],
                        avatar: None,
                    },
                ),
                (
                    "@bob:example.invalid".to_owned(),
                    UserProfile {
                        user_id: "@bob:example.invalid".to_owned(),
                        display_name: Some("Bob Upstream".to_owned()),
                        display_label: "Bob Alias".to_owned(),
                        original_display_label: "Bob Upstream".to_owned(),
                        mention_search_terms: vec![],
                        avatar: None,
                    },
                ),
                (
                    "@carol:example.invalid".to_owned(),
                    UserProfile {
                        user_id: "@carol:example.invalid".to_owned(),
                        display_name: Some("Carol Upstream".to_owned()),
                        display_label: "Carol Alias".to_owned(),
                        original_display_label: "Carol Upstream".to_owned(),
                        mention_search_terms: vec![],
                        avatar: None,
                    },
                ),
            ]),
            local_aliases: BTreeMap::from([
                (
                    "@alice:example.invalid".to_owned(),
                    "Alice Alias".to_owned(),
                ),
                ("@bob:example.invalid".to_owned(), "Bob Alias".to_owned()),
                (
                    "@carol:example.invalid".to_owned(),
                    "Carol Alias".to_owned(),
                ),
            ]),
            local_alias_update: LocalUserAliasUpdateState::Idle,
            update: Default::default(),
        };
        let (_snapshot_tx, snapshot_rx) = watch::channel(VersionedAppStateSnapshot {
            generation: 0,
            state,
        });
        let mut connection = CoreConnection {
            connection_id: RuntimeConnectionId(7),
            command_tx,
            event_rx,
            snapshot_rx,
            next_sequence: AtomicU64::new(1),
        };
        let key = TimelineKey {
            account_key: AccountKey("@me:example.invalid".to_owned()),
            kind: TimelineKind::Room {
                room_id: "!room:example.invalid".to_owned(),
            },
        };

        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: None,
            key,
            generation: crate::ids::TimelineGeneration(0),
            items: vec![TimelineItem {
                id: TimelineItemId::Event {
                    event_id: "$event:example.invalid".to_owned(),
                },
                sender: Some("@alice:example.invalid".to_owned()),
                sender_label: None,
                sender_avatar: None,
                body: Some("hello".to_owned()),
                notice_i18n_key: None,
                message_kind: Default::default(),
                spoiler_spans: Vec::new(),
                timestamp_ms: Some(1),
                in_reply_to_event_id: Some("$root:example.invalid".to_owned()),
                formatted: None,
                reply_quote: Some(koushi_state::ReplyQuote {
                    event_id: "$root:example.invalid".to_owned(),
                    sender: Some("@bob:example.invalid".to_owned()),
                    sender_label: None,
                    body_preview: Some("quoted".to_owned()),
                    state: koushi_state::ReplyQuoteState::Ready,
                }),
                thread_root: None,
                thread_summary: Some(ThreadSummaryDto {
                    reply_count: 1,
                    latest_event_id: Some("$latest:example.invalid".to_owned()),
                    latest_sender: Some("@carol:example.invalid".to_owned()),
                    latest_sender_label: None,
                    latest_body_preview: Some("latest".to_owned()),
                    latest_timestamp_ms: Some(2),
                }),
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
                actions: Default::default(),
                send_state: None,
                unable_to_decrypt: None,
            }],
        }));

        match connection.recv_event().await.expect("timeline event") {
            CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) => {
                let item = items.first().expect("projected item");
                assert_eq!(item.sender.as_deref(), Some("@alice:example.invalid"));
                assert_eq!(item.sender_label.as_deref(), Some("Alice Alias"));
                let quote = item.reply_quote.as_ref().expect("reply quote");
                assert_eq!(quote.sender.as_deref(), Some("@bob:example.invalid"));
                assert_eq!(quote.sender_label.as_deref(), Some("Bob Alias"));
                let thread = item.thread_summary.as_ref().expect("thread summary");
                assert_eq!(
                    thread.latest_sender.as_deref(),
                    Some("@carol:example.invalid")
                );
                assert_eq!(thread.latest_sender_label.as_deref(), Some("Carol Alias"));
            }
            other => panic!("expected projected timeline event, got {other:?}"),
        }

        let key = TimelineKey {
            account_key: AccountKey("@me:example.invalid".to_owned()),
            kind: TimelineKind::Room {
                room_id: "!room:example.invalid".to_owned(),
            },
        };
        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key,
            generation: crate::ids::TimelineGeneration(0),
            batch_id: crate::ids::TimelineBatchId(1),
            diffs: vec![TimelineDiff::PushBack {
                item: TimelineItem {
                    id: TimelineItemId::Event {
                        event_id: "$later:example.invalid".to_owned(),
                    },
                    sender: Some("@alice:example.invalid".to_owned()),
                    sender_label: None,
                    sender_avatar: None,
                    body: Some("later".to_owned()),
                    notice_i18n_key: None,
                    message_kind: Default::default(),
                    spoiler_spans: Vec::new(),
                    timestamp_ms: Some(3),
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
                    is_hidden: false,
                    can_redact: false,
                    is_edited: false,
                    can_edit: false,
                    actions: Default::default(),
                    send_state: None,
                    unable_to_decrypt: None,
                },
            }],
        }));

        match connection.recv_event().await.expect("timeline diff event") {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
                let TimelineDiff::PushBack { item } = diffs.first().expect("projected diff item")
                else {
                    panic!("expected push-back diff");
                };
                assert_eq!(item.sender.as_deref(), Some("@alice:example.invalid"));
                assert_eq!(item.sender_label.as_deref(), Some("Alice Alias"));
            }
            other => panic!("expected projected timeline diff event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn actor_profile_changes_emit_timeline_display_label_updates() {
        let runtime = CoreRuntime::start_with_event_capacity(8);
        let mut connection = runtime.attach();

        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),
                    user_id: "@me:example.invalid".to_owned(),
                    device_id: "DEVICE".to_owned(),
                }),
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
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),
                    user_id: "@me:example.invalid".to_owned(),
                    device_id: "DEVICE".to_owned(),
                }),
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
    fn app_actor_persistence_uses_blocking_store_port() {
        let source = include_str!("runtime.rs");
        let load_navigation = source
            .split("async fn load_navigation_for_current_session")
            .nth(1)
            .expect("navigation loader should exist")
            .split("async fn load_composer_drafts_for_current_session")
            .next()
            .expect("composer loader should follow navigation loader");
        let save_scheduled = source
            .split("async fn persist_scheduled_sends")
            .nth(1)
            .expect("scheduled persist helper should exist")
            .split("async fn persist_navigation")
            .next()
            .expect("navigation persist should follow scheduled persist");
        let save_navigation = source
            .split("async fn persist_navigation")
            .nth(1)
            .expect("navigation persist helper should exist")
            .split("async fn persist_room_preferences")
            .next()
            .expect("room preference persist should follow navigation persist");
        let save_preferences = source
            .split("async fn persist_room_preferences")
            .nth(1)
            .expect("room preference persist helper should exist")
            .split("async fn schedule_composer_draft_persist")
            .next()
            .expect("composer schedule should follow room preference persist");
        let flush_drafts = source
            .split("async fn flush_pending_composer_drafts")
            .nth(1)
            .expect("composer draft flush should exist")
            .split("fn scheduled_send_delay")
            .next()
            .expect("scheduled send delay should follow composer draft flush");
        let settings_effect = source
            .split("AppEffect::PersistSettings")
            .nth(1)
            .expect("settings persist effect should exist")
            .split("AppEffect::PersistRoomPreferences")
            .next()
            .expect("room preference effect should follow settings effect");

        for section in [
            load_navigation,
            save_scheduled,
            save_navigation,
            save_preferences,
            flush_drafts,
            settings_effect,
        ] {
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
            effects_helper.contains("TimelineCommand::EnsureSubscribed"),
            "room selection should ensure a room timeline exists"
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
    fn replacement_focused_helper_preserves_same_key_and_unsubscribes_different_key() {
        let account_key = AccountKey("@alice:example.invalid".to_owned());
        let current = TimelineKey {
            account_key: account_key.clone(),
            kind: TimelineKind::Focused {
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event-a:example.invalid".to_owned(),
            },
        };
        let same = current.clone();
        let different = TimelineKey {
            account_key,
            kind: TimelineKind::Focused {
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event-b:example.invalid".to_owned(),
            },
        };

        assert_eq!(
            unsubscribe_replaced_focused_context_timeline_key(Some(current.clone()), same),
            None
        );
        assert_eq!(
            unsubscribe_replaced_focused_context_timeline_key(Some(current.clone()), different),
            Some(current)
        );
        assert_eq!(
            unsubscribe_replaced_focused_context_timeline_key(
                None,
                focused_key("$event-c:example.invalid")
            ),
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
    fn oidc_completion_has_no_runtime_projection_before_actor_completion() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 8,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::CompleteOidcLogin {
                request_id,
                callback_url: "koushi-desktop://auth/callback?code=secret".to_owned(),
            }),
            None
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
    fn verification_followup_commands_project_flow_id_not_command_request_id() {
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
            Some(AppAction::VerificationCancelled {
                request_id: flow_id,
                reason: koushi_state::VerificationCancelReason::User,
            })
        );
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

    fn focused_key(event_id: &str) -> TimelineKey {
        TimelineKey {
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            kind: TimelineKind::Focused {
                room_id: "!room:example.invalid".to_owned(),
                event_id: event_id.to_owned(),
            },
        }
    }
}
