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

use matrix_desktop_state::{AppAction, AppState, reduce};
use tokio::sync::{broadcast, mpsc, watch};

use crate::account::{AccountActorHandle, AccountMessage};
use crate::command::{AppCommand, CoreCommand};
use crate::event::{AppStateSnapshot, CoreEvent};
use crate::executor;
use crate::failure::CoreFailure;
use crate::ids::{RequestId, RuntimeConnectionId};
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
        let (snapshot_tx, snapshot_rx) = watch::channel(AppState::default());
        let (action_tx, action_rx) = mpsc::channel(COMMAND_INBOX_CAPACITY);

        // Build the store actor (owns the credential store backend).
        let store_actor = StoreActor::new(data_dir);

        // Spawn AccountActor with shared channels.
        let account_actor = crate::account::AccountActor::spawn(
            store_actor,
            action_tx.clone(),
            event_tx.clone(),
        );

        let actor = AppActor {
            command_rx,
            action_rx,
            event_tx: event_tx.clone(),
            snapshot_tx,
            state: AppState::default(),
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
                        // Effects are executed by actors in later phases;
                        // the reducer remains the single UI state
                        // transition mechanism.
                        let _effects = reduce(&mut self.state, action);
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
                // Route to AccountActor; it will produce AppActions and
                // CoreEvents. AppActor does not immediately know the result —
                // it observes it via the action channel.
                let _ = self
                    .account_actor
                    .send(AccountMessage::Command(account_command))
                    .await;
                false
            }
            CoreCommand::App(app_command) => match app_command {
                AppCommand::Shutdown { .. } => false,
                AppCommand::SetComposerReplyTarget {
                    room_id,
                    event_id,
                    ..
                } => {
                    let effects = reduce(
                        &mut self.state,
                        AppAction::ComposerReplyTargetSelected { room_id, event_id },
                    );
                    let _ = effects;
                    true
                }
                AppCommand::CancelComposerReply { .. } => {
                    let effects = reduce(&mut self.state, AppAction::ComposerReplyCancelled);
                    let _ = effects;
                    true
                }
                AppCommand::OpenThread {
                    room_id,
                    root_event_id,
                    ..
                } => {
                    let effects = reduce(
                        &mut self.state,
                        AppAction::OpenThread {
                            room_id,
                            root_event_id,
                        },
                    );
                    let _ = effects;
                    true
                }
                AppCommand::CloseThread { .. } => {
                    let effects = reduce(&mut self.state, AppAction::CloseThread);
                    let _ = effects;
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
                // Route to AccountActor (which forwards to SearchActor).
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
}
