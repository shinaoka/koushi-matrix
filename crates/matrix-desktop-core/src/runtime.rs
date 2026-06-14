//! `CoreRuntime`, `CoreConnection`, and the `AppActor` loop.
//!
//! Channel topology (overview.md, Async rule 10):
//! - command inbox per runtime: bounded mpsc, capacity 256
//! - discrete core events per consumer: broadcast, capacity 1024; a lagged
//!   consumer observes `EventStreamLag` and resyncs from the snapshot watch
//! - state snapshots: latest-wins watch, coalesced to at most one
//!   `StateChanged` per processed command batch

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use matrix_desktop_state::{
    AppAction, AppEffect, AppState, SearchScope as AppSearchScope, SessionState, ThreadPaneState,
    reduce,
};
use tokio::sync::{broadcast, mpsc, watch};

use crate::account::{AccountActorHandle, AccountMessage};
use crate::command::{
    AccountCommand, AppCommand, CoreCommand, SearchCommand, SearchScope, TimelineCommand,
};
use crate::event::{AppStateSnapshot, CoreEvent};
use crate::executor;
use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use crate::settings::SettingsStore;
use crate::store::StoreActor;

pub const COMMAND_INBOX_CAPACITY: usize = 256;
pub const EVENT_QUEUE_CAPACITY: usize = 1024;

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
    snapshot_rx: watch::Receiver<AppStateSnapshot>,
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
        Self::start_inner(EVENT_QUEUE_CAPACITY, data_dir)
    }

    #[cfg(test)]
    pub(crate) fn start_with_event_capacity(event_capacity: usize) -> Self {
        Self::start_inner(event_capacity, default_data_dir())
    }

    fn start_inner(event_capacity: usize, data_dir: PathBuf) -> Self {
        let (command_tx, command_rx) = mpsc::channel(COMMAND_INBOX_CAPACITY);
        let (event_tx, _) = broadcast::channel(event_capacity);
        let (action_tx, action_rx) = mpsc::channel(COMMAND_INBOX_CAPACITY);
        let settings_store = SettingsStore::new(&data_dir);

        let mut initial_state = AppState::default();
        let settings_action = match settings_store.load() {
            Ok(values) => AppAction::SettingsLoaded { values },
            Err(_) => AppAction::SettingsLoadFailed {
                message: "settings could not be loaded".to_owned(),
            },
        };
        let _ = reduce(&mut initial_state, settings_action);
        let (snapshot_tx, snapshot_rx) = watch::channel(initial_state.clone());

        // Build the store actor (owns the credential store backend).
        let store_actor = StoreActor::new(data_dir);

        // Spawn AccountActor with shared channels.
        let account_actor =
            crate::account::AccountActor::spawn(store_actor, action_tx.clone(), event_tx.clone());

        let actor = AppActor {
            command_rx,
            action_rx,
            event_tx: event_tx.clone(),
            snapshot_tx,
            state: initial_state,
            settings_store,
            account_actor,
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
    snapshot_rx: watch::Receiver<AppStateSnapshot>,
    next_sequence: AtomicU64,
}

impl CoreConnection {
    pub fn connection_id(&self) -> RuntimeConnectionId {
        self.connection_id
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
        if command.request_id().connection_id != self.connection_id {
            return Err(CommandSubmitError::InvalidRequestId);
        }
        self.command_tx
            .send(command)
            .await
            .map_err(|_| CommandSubmitError::RuntimeClosed)
    }

    /// Receive the next event. On lag, intermediate events were dropped for
    /// this consumer; resync from [`Self::snapshot`].
    pub async fn recv_event(&mut self) -> Result<CoreEvent, EventStreamLag> {
        loop {
            match self.event_rx.recv().await {
                Ok(event) => return Ok(event),
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

    /// Latest state snapshot (latest-wins watch semantics).
    pub fn snapshot(&self) -> AppStateSnapshot {
        self.snapshot_rx.borrow().clone()
    }
}

struct AppActor {
    command_rx: mpsc::Receiver<CoreCommand>,
    action_rx: mpsc::Receiver<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    snapshot_tx: watch::Sender<AppStateSnapshot>,
    state: AppState,
    settings_store: SettingsStore,
    account_actor: AccountActorHandle,
}

impl AppActor {
    async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break };
                    let mut state_changed = self.handle_command(command).await;
                    // Coalesce: drain whatever is already queued before
                    // emitting a single StateChanged for the batch.
                    while let Ok(next) = self.command_rx.try_recv() {
                        state_changed |= self.handle_command(next).await;
                    }
                    if state_changed {
                        self.publish_snapshot();
                    }
                }
                actions = self.action_rx.recv() => {
                    let Some(actions) = actions else { break };
                    let mut state_changed = false;
                    for action in actions {
                        // Actor-originated actions are post-side-effect
                        // projections: the owner actor has already performed
                        // the corresponding Matrix/store/sync operation.
                        // AppActor owns AppCommand effects above; replaying
                        // actor-projection effects here would double-execute
                        // login, restore, sync, or recovery work.
                        let _post_projection_effects = reduce(&mut self.state, action);
                        state_changed = true;
                    }
                    if state_changed {
                        self.publish_snapshot();
                    }
                }
            }
        }
        // Shutdown: tell AccountActor to stop.
        let _ = self.account_actor.send(AccountMessage::Shutdown).await;
    }

    /// Returns whether `AppState` changed.
    async fn handle_command(&mut self, command: CoreCommand) -> bool {
        if command.requires_ready_session()
            && !matches!(
                self.state.session,
                matrix_desktop_state::SessionState::Ready(_)
            )
        {
            self.emit(CoreEvent::OperationFailed {
                request_id: command.request_id(),
                failure: CoreFailure::SessionRequired,
            });
            return false;
        }

        match command {
            CoreCommand::Account(account_command) => {
                let effects = account_command_projected_action(&account_command)
                    .map(|action| reduce(&mut self.state, action))
                    .unwrap_or_default();
                let projected_state_changed = !effects.is_empty();
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
                    let effects = reduce(
                        &mut self.state,
                        AppAction::ComposerReplyTargetSelected { room_id, event_id },
                    );
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::CancelComposerReply { request_id } => {
                    let effects = reduce(&mut self.state, AppAction::ComposerReplyCancelled);
                    self.handle_app_effects(request_id, effects).await;
                    true
                }
                AppCommand::SetThreadComposerDraft {
                    request_id,
                    room_id,
                    root_event_id,
                    draft,
                } => {
                    let effects = reduce(
                        &mut self.state,
                        AppAction::ThreadComposerDraftChanged {
                            room_id,
                            root_event_id,
                            draft,
                        },
                    );
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
                    let effects = reduce(
                        &mut self.state,
                        AppAction::OpenThread {
                            room_id,
                            root_event_id,
                        },
                    );
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
                    let effects = reduce(&mut self.state, AppAction::CloseThread);
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
                    let replaced_focused_key =
                        self.unsubscribe_replaced_focused_context_timeline(&room_id, &event_id);
                    let effects = reduce(
                        &mut self.state,
                        AppAction::OpenFocusedContext { room_id, event_id },
                    );
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
                AppCommand::CloseFocusedContext { request_id } => {
                    let focused_key = self.current_focused_context_timeline_key();
                    let effects = reduce(&mut self.state, AppAction::CloseFocusedContext);
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
                AppCommand::UpdateSettings { request_id, patch } => {
                    let effects = reduce(
                        &mut self.state,
                        AppAction::SettingsUpdateRequested {
                            request_id: request_id.sequence,
                            patch,
                        },
                    );
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
                // Route to AccountActor (which forwards to RoomActor).
                let _ = self
                    .account_actor
                    .send(crate::account::AccountMessage::RoomCommand(room_command))
                    .await;
                false
            }
            CoreCommand::Timeline(timeline_command) => {
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
                let (request_id, query, scope) = match search_command {
                    SearchCommand::Query {
                        request_id,
                        query,
                        scope,
                    } => (request_id, query, scope),
                };
                let effects = reduce(
                    &mut self.state,
                    AppAction::SearchSubmitted {
                        request_id: request_id.sequence,
                        query: query.clone(),
                        scope: map_core_search_scope_to_state(scope.clone()),
                    },
                );
                self.handle_app_effects(request_id, effects).await;
                true
            }
        }
    }

    async fn handle_app_effects(&mut self, request_id: RequestId, effects: Vec<AppEffect>) {
        for effect in effects {
            if let AppEffect::OpenThreadTimeline {
                room_id,
                root_event_id,
            } = effect
            {
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
            } else if let AppEffect::OpenFocusedTimeline { room_id, event_id } = effect {
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
            } else if let AppEffect::SearchMessages {
                request_id: effect_request_id,
                query,
                scope,
            } = effect
            {
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
            } else if let AppEffect::PersistSettings {
                request_id: effect_request_id,
                values,
            } = effect
            {
                if effect_request_id != request_id.sequence {
                    continue;
                }
                let action = match self.settings_store.save(&values) {
                    Ok(()) => AppAction::SettingsPersisted {
                        request_id: effect_request_id,
                    },
                    Err(_) => AppAction::SettingsPersistFailed {
                        request_id: effect_request_id,
                        message: "settings could not be saved".to_owned(),
                    },
                };
                let _ = reduce(&mut self.state, action);
            }
        }
    }

    async fn send_timeline_command_or_fail(&self, request_id: RequestId, command: TimelineCommand) {
        if !self
            .account_actor
            .send(AccountMessage::TimelineCommand(command))
            .await
        {
            self.emit(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            });
        }
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
            matrix_desktop_state::FocusedContextState::Opening { room_id, event_id }
            | matrix_desktop_state::FocusedContextState::Open {
                room_id, event_id, ..
            } => Some(TimelineKey {
                account_key,
                kind: TimelineKind::Focused {
                    room_id: room_id.clone(),
                    event_id: event_id.clone(),
                },
            }),
            matrix_desktop_state::FocusedContextState::Closed => None,
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

    fn publish_snapshot(&self) {
        let _ = self.snapshot_tx.send(self.state.clone());
        let _ = self
            .event_tx
            .send(CoreEvent::StateChanged(self.state.clone()));
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
        SearchScope::Global => AppSearchScope::AllRooms,
        SearchScope::Room { room_id } => AppSearchScope::CurrentRoom { room_id },
    }
}

fn account_command_projected_action(command: &AccountCommand) -> Option<AppAction> {
    match command {
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
        AccountCommand::BootstrapCrossSigning { request_id } => {
            Some(AppAction::BootstrapCrossSigningRequested {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::EnableKeyBackup { request_id } => {
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
        AccountCommand::ResetIdentity { request_id } => Some(AppAction::ResetIdentityRequested {
            request_id: request_id.sequence,
        }),
        AccountCommand::SubmitIdentityResetAuth { request_id, .. } => {
            Some(AppAction::ResetIdentityAuthSubmitted {
                request_id: request_id.sequence,
            })
        }
        AccountCommand::LoginPassword { .. }
        | AccountCommand::RestoreSession { .. }
        | AccountCommand::RestoreLastSession { .. }
        | AccountCommand::QuerySavedSessions { .. }
        | AccountCommand::SubmitRecovery { .. }
        | AccountCommand::Logout { .. }
        | AccountCommand::SwitchAccount { .. } => None,
    }
}

fn map_state_search_scope_to_core(scope: AppSearchScope) -> SearchScope {
    match scope {
        AppSearchScope::AllRooms | AppSearchScope::CurrentSpace { .. } | AppSearchScope::Dms => {
            SearchScope::Global
        }
        AppSearchScope::CurrentRoom { room_id } => SearchScope::Room { room_id },
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
        home.ok_or_else(|| "HOME is required to resolve matrix-desktop data dir".to_owned())?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("matrix-desktop"))
}

/// Default application data directory (`$HOME/.local/share/matrix-desktop`).
fn default_data_dir() -> PathBuf {
    default_data_dir_from_home(std::env::var_os("HOME"))
        .expect("HOME is required to resolve matrix-desktop data dir")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_data_dir_requires_home() {
        assert!(default_data_dir_from_home(None).is_err());
    }

    #[test]
    fn default_data_dir_uses_xdg_like_user_data_path() {
        let dir = default_data_dir_from_home(Some("/tmp/synthetic-home".into())).unwrap();
        assert!(dir.ends_with(".local/share/matrix-desktop"));
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
    fn identity_reset_auth_command_projects_pending_state_before_routing() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 7,
        };

        assert_eq!(
            account_command_projected_action(&AccountCommand::SubmitIdentityResetAuth {
                request_id,
                request: matrix_desktop_state::IdentityResetAuthRequest::OAuthApproved,
            }),
            Some(AppAction::ResetIdentityAuthSubmitted { request_id: 7 })
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
                request: matrix_desktop_state::RecoveryRequest {
                    secret: matrix_desktop_state::AuthSecret::new("recovery secret"),
                },
            }),
            Some(AppAction::RestoreKeyBackupRequested {
                request_id: 9,
                version: Some("backup-version-1".to_owned()),
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
                reason: matrix_desktop_state::VerificationCancelReason::User,
            }),
            Some(AppAction::VerificationCancelled {
                request_id: flow_id,
                reason: matrix_desktop_state::VerificationCancelReason::User,
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
