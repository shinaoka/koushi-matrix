//! RoomActor: room list normalization and room operations.
//!
//! ## Ownership
//! `RoomActor` is owned by `AccountActor`. Its task handle lives inside
//! `AccountActor`; colocated as a child task per the spec
//! ("Actor Deployment And Supervision — boundaries define ownership, not one
//! task per actor").
//!
//! ## Room list normalization (canon: overview.md RoomActor bullet)
//! Constructing ad-hoc `RoomListService` instances is PROHIBITED: they are
//! not driven by the sync loop, race the running `SyncService`, and return
//! entries without the live service's `required_state` (e.g. `m.room.create`
//! for space classification — deterministically broken on Conduit).
//!
//! `RoomMessage::SyncStarted` carries the backend handle:
//! - `Some(Arc<RoomListService>)` on the SyncService backend — the ONE live
//!   service owned by the running `SyncService` (`sync_service
//!   .room_list_service()`). The actor subscribes to its `all_rooms()`
//!   entries stream (`entries_with_dynamic_adapters` with the non-left filter)
//!   and KEEPS CONSUMING it, re-normalizing on each joined/invited diff batch
//!   (Async rule 1: actors relay the SDK's observable streams).
//! - `None` on the LegacySync backend — the actor normalizes from
//!   `client.joined_rooms()` and relays `client
//!   .subscribe_to_all_room_updates()` (which fires on the legacy backend
//!   because it feeds the base client), coalescing pending batches into one
//!   re-normalization per wakeup.
//!
//! Snapshots are projected as `AppAction::RoomListUpdated` +
//! `RoomEvent::RoomListUpdated`.
//!
//! Operation-triggered refreshes after the actor's own mutations remain: on
//! the SyncService path "refresh" means "re-normalize from the live service's
//! current entries" (a refresh request to the observation loop), never "new
//! service"; on the LegacySync path it is a joined_rooms re-normalization.
//!
//! Per Async rule 9: "Because the local QA matrix includes homeservers without
//! MSC4186, this legacy room-list path is a fully implemented, QA-gated
//! product path, not a stub."
//!
//! ## Room operations
//! `CreateRoom`, `CreateSpace`, `SetSpaceChild`, `InviteUser`, `JoinRoom`,
//! `LeaveRoom`, and `ForgetRoom` call `matrix-desktop-sdk` primitives and emit
//! domain events with `request_id`. Errors are classified into
//! `RoomFailureKind` (never raw SDK text).
//!
//! ## SelectSpace / SelectRoom
//! Pure navigation — project `AppAction::SelectSpace` / `AppAction::SelectRoom`
//! through the action channel. Core applies the navigation state update here
//! and does not consume reducer effects in this actor.
//!
//! ## Security
//! Raw SDK error text never appears in events or AppState. All errors are
//! classified into `RoomFailureKind`.

use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
};

use matrix_desktop_sdk::{
    MatrixClientSession, MatrixPublicRoomDirectoryQuery, MatrixPublicRoomDirectoryRoom,
    MatrixRoomOperationError, MatrixRoomTagKind, MatrixRoomTags,
};
use matrix_desktop_state::{
    AppAction, AvatarImage, AvatarThumbnailState, BasicOperationRequest, DirectoryQuery,
    DirectoryRoomSummary, InvitePreview, OperationFailureKind, PinnedEvent, RoomSummary,
    RoomTagInfo, RoomTagKind, RoomTags, SpaceSummary,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::command::RoomCommand;
use crate::event::{CoreEvent, RoomEvent};
use crate::executor;
use crate::failure::{CoreFailure, RoomFailureKind};
use crate::ids::RequestId;

/// Fixed, content-free messages recorded in `AppState.errors` when a basic
/// operation fails. Raw SDK errors are classified into `RoomFailureKind` for the
/// transport `OperationFailed` event and never placed in product state.
const CREATE_ROOM_FAILED_MESSAGE: &str = "Room creation failed";
const CREATE_SPACE_FAILED_MESSAGE: &str = "Space creation failed";
const LINK_SPACE_CHILD_FAILED_MESSAGE: &str = "Linking the room to the space failed";

/// Messages sent to the RoomActor from AccountActor / SyncActor.
pub enum RoomMessage {
    /// Route a `RoomCommand` to the actor.
    Command(RoomCommand),
    /// A store-backed session was established (login/restore/switch).
    /// Enables room operations; does NOT start the room-list observation —
    /// that starts on `SyncStarted` when the backend (and its live
    /// `RoomListService`, if any) is known.
    SessionEstablished { session: Arc<MatrixClientSession> },
    /// Sync started. Sent by `SyncActor` after the backend is launched.
    /// `room_list_service` is the ONE live service owned by the running
    /// `SyncService` (`Some` on the SyncService backend, `None` on
    /// LegacySync). Ad-hoc `RoomListService` instances are prohibited
    /// (canon, overview.md RoomActor bullet).
    SyncStarted {
        session: Arc<MatrixClientSession>,
        room_list_service: Option<Arc<matrix_sdk_ui::room_list_service::RoomListService>>,
    },
    /// Sync stopped: tear down any active room list subscription.
    SyncStopped,
    /// Ordered shutdown.
    Shutdown,
}

/// Handle to the RoomActor background task (owned by AccountActor).
pub struct RoomActorHandle {
    pub(crate) tx: mpsc::Sender<RoomMessage>,
    task: executor::JoinHandle<()>,
}

impl RoomActorHandle {
    pub async fn send(&self, msg: RoomMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Non-blocking send for use in sync contexts (e.g. `spawn_sync_actor`
    /// which is a `fn` not `async fn`). Returns false if the channel is full
    /// or closed.
    pub(crate) fn try_send(&self, msg: RoomMessage) -> bool {
        self.tx.try_send(msg).is_ok()
    }

    /// Wait for the actor task to complete (used in ordered shutdown).
    pub async fn join(self) {
        let _ = self.task.await;
    }
}

/// Handle on the spawned room-list observation loop: oneshot stop signal plus
/// the task handle so teardown can await completion (same pattern as
/// `sync.rs` `legacy_stop_tx`). `refresh_tx` is `Some` on the SyncService
/// (live-service) loop: an operation-triggered refresh re-normalizes from the
/// live service's current entries inside the loop.
struct RoomListObservation {
    stop_tx: oneshot::Sender<()>,
    task: executor::JoinHandle<()>,
    refresh_tx: Option<mpsc::Sender<()>>,
}

pub struct RoomActor {
    session: Option<Arc<MatrixClientSession>>,
    observation: Option<RoomListObservation>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    command_rx: mpsc::Receiver<RoomMessage>,
}

impl RoomActor {
    pub fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
    ) -> RoomActorHandle {
        let (tx, command_rx) = mpsc::channel(64);
        let actor = RoomActor {
            session: None,
            observation: None,
            known_room_ids: Arc::new(RwLock::new(BTreeSet::new())),
            action_tx,
            event_tx,
            command_rx,
        };
        let task = executor::spawn(actor.run());
        RoomActorHandle { tx, task }
    }

    async fn run(mut self) {
        while let Some(msg) = self.command_rx.recv().await {
            match msg {
                RoomMessage::Shutdown => {
                    self.stop_observation().await;
                    break;
                }
                RoomMessage::Command(command) => {
                    self.handle_command(command).await;
                }
                RoomMessage::SessionEstablished { session } => {
                    // Room operations become available; observation starts
                    // later on SyncStarted (backend then known).
                    self.session = Some(session);
                    self.clear_known_rooms();
                }
                RoomMessage::SyncStarted {
                    session,
                    room_list_service,
                } => {
                    // Guard against two observation loops running: a previous
                    // loop (from an earlier SyncStarted) is stopped before the
                    // replacement is spawned.
                    self.stop_observation().await;
                    self.session = Some(session.clone());
                    self.clear_known_rooms();
                    match room_list_service {
                        Some(service) => {
                            // SyncService backend: relay the live service's
                            // entries stream. Its first diff batch (Reset with
                            // the current entries) provides the initial
                            // snapshot, so no separate initial refresh is
                            // needed.
                            self.start_live_observation(session, service);
                        }
                        None => {
                            // LegacySync backend: initial snapshot from
                            // joined_rooms, then relay the base client's room
                            // update broadcast (Async rule 1).
                            self.refresh_room_list().await;
                            self.start_legacy_observation();
                        }
                    }
                }
                RoomMessage::SyncStopped => {
                    self.stop_observation().await;
                    self.clear_known_rooms();
                }
            }
        }
    }

    /// Spawn the live-service observation loop (SyncService backend): relay
    /// the ONE live `RoomListService`'s entries stream and re-normalize on
    /// each diff batch.
    fn start_live_observation(
        &mut self,
        session: Arc<MatrixClientSession>,
        service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    ) {
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (refresh_tx, refresh_rx) = mpsc::channel::<()>(8);
        let task = executor::spawn(run_live_room_list_observation(
            session,
            service,
            self.known_room_ids.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            refresh_rx,
            stop_rx,
        ));
        self.observation = Some(RoomListObservation {
            stop_tx,
            task,
            refresh_tx: Some(refresh_tx),
        });
    }

    /// Spawn the legacy room-list observation loop (LegacySync backend) for
    /// the current session.
    fn start_legacy_observation(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let task = executor::spawn(run_legacy_room_list_observation(
            session.clone(),
            self.known_room_ids.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            stop_rx,
        ));
        self.observation = Some(RoomListObservation {
            stop_tx,
            task,
            refresh_tx: None,
        });
    }

    /// Stop the observation loop (if running) and wait for it to exit.
    async fn stop_observation(&mut self) {
        if let Some(observation) = self.observation.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn handle_command(&self, command: RoomCommand) {
        match command {
            RoomCommand::CreateRoom {
                request_id,
                name,
                encrypted,
            } => {
                self.handle_create_room(request_id, name, encrypted).await;
            }
            RoomCommand::CreatePublicDirectoryRoom {
                request_id,
                name,
                alias_localpart,
            } => {
                self.handle_create_public_directory_room(request_id, name, alias_localpart)
                    .await;
            }
            RoomCommand::CreateSpace { request_id, name } => {
                self.handle_create_space(request_id, name).await;
            }
            RoomCommand::SetSpaceChild {
                request_id,
                space_id,
                child_room_id,
                via_server,
            } => {
                self.handle_set_space_child(request_id, space_id, child_room_id, via_server)
                    .await;
            }
            RoomCommand::InviteUser {
                request_id,
                room_id,
                user_id,
            } => {
                self.handle_invite_user(request_id, room_id, user_id).await;
            }
            RoomCommand::AcceptInvite {
                request_id,
                room_id,
            } => {
                self.handle_accept_invite(request_id, room_id).await;
            }
            RoomCommand::DeclineInvite {
                request_id,
                room_id,
            } => {
                self.handle_decline_invite(request_id, room_id).await;
            }
            RoomCommand::StartDirectMessage {
                request_id,
                user_id,
            } => {
                self.handle_start_direct_message(request_id, user_id).await;
            }
            RoomCommand::JoinRoom {
                request_id,
                room_id,
            } => {
                self.handle_join_room(request_id, room_id).await;
            }
            RoomCommand::LeaveRoom {
                request_id,
                room_id,
            } => {
                self.handle_leave_room(request_id, room_id).await;
            }
            RoomCommand::ForgetRoom {
                request_id,
                room_id,
            } => {
                self.handle_forget_room(request_id, room_id).await;
            }
            RoomCommand::SetTag {
                request_id,
                room_id,
                tag,
                order,
            } => {
                self.handle_set_tag(request_id, room_id, tag, order).await;
            }
            RoomCommand::RemoveTag {
                request_id,
                room_id,
                tag,
            } => {
                self.handle_remove_tag(request_id, room_id, tag).await;
            }
            RoomCommand::PinEvent {
                request_id,
                room_id,
                event_id,
            } => {
                self.handle_pin_event(request_id, room_id, event_id).await;
            }
            RoomCommand::UnpinEvent {
                request_id,
                room_id,
                event_id,
            } => {
                self.handle_unpin_event(request_id, room_id, event_id).await;
            }
            RoomCommand::QueryDirectory { request_id, query } => {
                self.handle_query_directory(request_id, query).await;
            }
            RoomCommand::JoinDirectoryRoom {
                request_id,
                alias,
                via_server,
            } => {
                self.handle_join_directory_room(request_id, alias, via_server)
                    .await;
            }
            RoomCommand::SelectSpace {
                request_id: _,
                space_id,
            } => {
                // Pure navigation: project to reducer; no domain event.
                // request_id correlation via StateChanged is implicit per spec.
                self.reduce(vec![AppAction::SelectSpace { space_id }]);
            }
            RoomCommand::SelectRoom {
                request_id: _,
                room_id,
            } => {
                // Pure navigation: project to reducer; no domain event.
                // Core updates navigation state here and does not consume
                // reducer effects in this actor.
                self.reduce(vec![AppAction::SelectRoom { room_id }]);
            }
        }
    }

    async fn handle_create_room(&self, request_id: RequestId, name: String, encrypted: bool) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        // Drive the basic-operation state machine: Idle -> CreatingRoom. The
        // reducer guards re-entry; `request_id.sequence` is the correlation id
        // the settle action below must match.
        self.reduce(vec![AppAction::BasicOperationRequested {
            request_id: request_id.sequence,
            request: BasicOperationRequest::CreateRoom { name: name.clone() },
        }]);
        match matrix_desktop_sdk::create_room(session, &name, encrypted).await {
            Ok(room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomCreated {
                    request_id,
                    room_id,
                }));
                self.reduce(vec![AppAction::BasicOperationSucceeded {
                    request_id: request_id.sequence,
                }]);
                // Reflect the actor's own mutation immediately instead of
                // waiting for the next sync round-trip.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                self.reduce(vec![AppAction::BasicOperationFailed {
                    request_id: request_id.sequence,
                    message: CREATE_ROOM_FAILED_MESSAGE.to_owned(),
                }]);
            }
        }
    }

    async fn handle_create_public_directory_room(
        &self,
        request_id: RequestId,
        name: String,
        alias_localpart: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match matrix_desktop_sdk::create_public_directory_room(session, &name, &alias_localpart)
            .await
        {
            Ok(room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomCreated {
                    request_id,
                    room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_create_space(&self, request_id: RequestId, name: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        // Drive the basic-operation state machine: Idle -> CreatingSpace.
        self.reduce(vec![AppAction::BasicOperationRequested {
            request_id: request_id.sequence,
            request: BasicOperationRequest::CreateSpace { name: name.clone() },
        }]);
        match matrix_desktop_sdk::create_space(session, &name).await {
            Ok(space_id) => {
                self.emit(CoreEvent::Room(RoomEvent::SpaceCreated {
                    request_id,
                    space_id,
                }));
                self.reduce(vec![AppAction::BasicOperationSucceeded {
                    request_id: request_id.sequence,
                }]);
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                self.reduce(vec![AppAction::BasicOperationFailed {
                    request_id: request_id.sequence,
                    message: CREATE_SPACE_FAILED_MESSAGE.to_owned(),
                }]);
            }
        }
    }

    async fn handle_set_space_child(
        &self,
        request_id: RequestId,
        space_id: String,
        child_room_id: String,
        via_server: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        // Drive the basic-operation state machine: Idle -> LinkingSpaceChild.
        self.reduce(vec![AppAction::BasicOperationRequested {
            request_id: request_id.sequence,
            request: BasicOperationRequest::LinkSpaceChild {
                space_id: space_id.clone(),
                child_room_id: child_room_id.clone(),
            },
        }]);
        match matrix_desktop_sdk::set_space_child(session, &space_id, &child_room_id, &via_server)
            .await
        {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::SpaceChildSet {
                    request_id,
                    space_id,
                    child_room_id,
                }));
                self.reduce(vec![AppAction::BasicOperationSucceeded {
                    request_id: request_id.sequence,
                }]);
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                self.reduce(vec![AppAction::BasicOperationFailed {
                    request_id: request_id.sequence,
                    message: LINK_SPACE_CHILD_FAILED_MESSAGE.to_owned(),
                }]);
            }
        }
    }

    async fn handle_invite_user(&self, request_id: RequestId, room_id: String, user_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::invite_user_to_room(session, &room_id, &user_id).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::UserInvited {
                    request_id,
                    room_id,
                    user_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_accept_invite(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::join_room_by_id(session, &room_id).await {
            Ok(joined_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::InviteAccepted {
                    request_id,
                    room_id: joined_room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_decline_invite(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::leave_room(session, &room_id).await {
            Ok(declined_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::InviteDeclined {
                    request_id,
                    room_id: declined_room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_start_direct_message(&self, request_id: RequestId, user_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::start_direct_message(session, &user_id).await {
            Ok(room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::DirectMessageStarted {
                    request_id,
                    room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_join_room(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::join_room_by_id(session, &room_id).await {
            Ok(joined_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomJoined {
                    request_id,
                    room_id: joined_room_id,
                }));
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_query_directory(&self, request_id: RequestId, query: DirectoryQuery) {
        self.reduce(vec![AppAction::DirectoryQueryRequested {
            request_id: request_id.sequence,
            query: query.clone(),
        }]);
        let Some(session) = &self.session else {
            self.reduce(vec![AppAction::DirectoryQueryFailed {
                request_id: request_id.sequence,
                query,
                kind: OperationFailureKind::Sdk,
            }]);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let sdk_query = MatrixPublicRoomDirectoryQuery {
            term: query.term.clone(),
            server_name: query.server_name.clone(),
            limit: query.limit,
            since: query.since.clone(),
        };
        match matrix_desktop_sdk::query_public_room_directory(session, sdk_query).await {
            Ok(result) => {
                let rooms: Vec<DirectoryRoomSummary> = result
                    .rooms
                    .into_iter()
                    .map(directory_room_summary_from_sdk)
                    .collect();
                self.reduce(vec![AppAction::DirectoryQuerySucceeded {
                    request_id: request_id.sequence,
                    query: query.clone(),
                    rooms: rooms.clone(),
                    next_batch: result.next_batch.clone(),
                }]);
                self.emit(CoreEvent::Room(RoomEvent::DirectoryQueryCompleted {
                    request_id,
                    query,
                    rooms,
                    next_batch: result.next_batch,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce(vec![AppAction::DirectoryQueryFailed {
                    request_id: request_id.sequence,
                    query,
                    kind: operation_failure_kind(kind),
                }]);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_join_directory_room(
        &self,
        request_id: RequestId,
        alias: String,
        via_server: Option<String>,
    ) {
        self.reduce(vec![AppAction::DirectoryJoinRequested {
            request_id: request_id.sequence,
            alias: alias.clone(),
            via_server: via_server.clone(),
        }]);
        let Some(session) = &self.session else {
            self.reduce(vec![AppAction::DirectoryJoinFailed {
                request_id: request_id.sequence,
                alias,
                via_server,
                kind: OperationFailureKind::Sdk,
            }]);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match matrix_desktop_sdk::join_room_by_alias(session, &alias, via_server.as_deref()).await {
            Ok(room_id) => {
                self.reduce(vec![AppAction::DirectoryJoinSucceeded {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                }]);
                self.emit(CoreEvent::Room(RoomEvent::RoomJoined {
                    request_id,
                    room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce(vec![AppAction::DirectoryJoinFailed {
                    request_id: request_id.sequence,
                    alias,
                    via_server,
                    kind: operation_failure_kind(kind),
                }]);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_leave_room(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::leave_room(session, &room_id).await {
            Ok(left_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomLeft {
                    request_id,
                    room_id: left_room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_forget_room(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::forget_room(session, &room_id).await {
            Ok(forgotten_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomForgotten {
                    request_id,
                    room_id: forgotten_room_id,
                }));
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_set_tag(
        &self,
        request_id: RequestId,
        room_id: String,
        tag: RoomTagKind,
        order: Option<f64>,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::set_room_tag(session, &room_id, sdk_room_tag_kind(tag), order)
            .await
        {
            Ok(()) => {
                let info = room_tag_info_from_order(order);
                if self
                    .action_tx
                    .send(vec![AppAction::RoomTagSet {
                        room_id: room_id.clone(),
                        tag,
                        info,
                    }])
                    .await
                    .is_err()
                {
                    self.emit_failure(
                        request_id,
                        CoreFailure::RoomOperationFailed {
                            kind: RoomFailureKind::Sdk,
                        },
                    );
                    return;
                }
                // `set_is_favourite` / `set_is_low_priority` only send the
                // tag mutation to the server; the SDK room-list snapshot may
                // remain stale until the next sync. Keep the immediate state
                // projection in the reducer action above instead of refreshing
                // and potentially overwriting it with old tags.
                self.emit(CoreEvent::Room(RoomEvent::RoomTagSet {
                    request_id,
                    room_id,
                    tag,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_remove_tag(&self, request_id: RequestId, room_id: String, tag: RoomTagKind) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_sdk::remove_room_tag(session, &room_id, sdk_room_tag_kind(tag)).await {
            Ok(()) => {
                if self
                    .action_tx
                    .send(vec![AppAction::RoomTagRemoved {
                        room_id: room_id.clone(),
                        tag,
                    }])
                    .await
                    .is_err()
                {
                    self.emit_failure(
                        request_id,
                        CoreFailure::RoomOperationFailed {
                            kind: RoomFailureKind::Sdk,
                        },
                    );
                    return;
                }
                // See `handle_set_tag`: the reducer owns the immediate state
                // projection, while the next sync snapshot becomes canonical.
                self.emit(CoreEvent::Room(RoomEvent::RoomTagRemoved {
                    request_id,
                    room_id,
                    tag,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_pin_event(&self, request_id: RequestId, room_id: String, event_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.reduce(vec![AppAction::PinEventRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        }]);
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match matrix_desktop_sdk::pin_event(session, &room_id, &event_id).await {
            Ok(()) => {
                self.reduce(vec![AppAction::PinEventCompleted {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                }]);
                self.emit(CoreEvent::Room(RoomEvent::PinEventCompleted {
                    request_id,
                    room_id: room_id.clone(),
                }));
                self.project_pinned_events_after_success(request_id, room_id)
                    .await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce(vec![AppAction::PinEventFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }]);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_unpin_event(&self, request_id: RequestId, room_id: String, event_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.reduce(vec![AppAction::UnpinEventRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        }]);
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match matrix_desktop_sdk::unpin_event(session, &room_id, &event_id).await {
            Ok(()) => {
                self.reduce(vec![AppAction::UnpinEventCompleted {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                }]);
                self.emit(CoreEvent::Room(RoomEvent::UnpinEventCompleted {
                    request_id,
                    room_id: room_id.clone(),
                }));
                self.project_pinned_events_after_success(request_id, room_id)
                    .await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce(vec![AppAction::UnpinEventFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }]);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn project_pinned_events_after_success(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            return;
        };
        let pinned = match matrix_desktop_sdk::load_pinned_event_ids(session, &room_id).await {
            Ok(event_ids) => pinned_events_from_ids(event_ids),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };

        self.reduce(vec![AppAction::RoomPinnedEventsUpdated {
            room_id: room_id.clone(),
            pinned: pinned.clone(),
        }]);
        self.emit(CoreEvent::Room(RoomEvent::PinnedEventsUpdated {
            room_id,
            pinned,
        }));
    }

    /// Refresh the room list and project it into AppState via the action
    /// channel. Also emits `RoomEvent::RoomListUpdated` as a discrete event.
    ///
    /// On the SyncService path this requests a re-normalization from the live
    /// service's current entries (inside the observation loop) — NEVER a new
    /// `RoomListService`. On the LegacySync path (or before sync starts) it
    /// re-normalizes from `client.joined_rooms()`.
    async fn refresh_room_list(&self) {
        if let Some(observation) = &self.observation
            && let Some(refresh_tx) = &observation.refresh_tx
        {
            let _ = refresh_tx.try_send(());
            return;
        }
        if let Some(session) = &self.session {
            refresh_room_list_from_joined_rooms(
                session,
                &self.known_room_ids,
                &self.action_tx,
                &self.event_tx,
            )
            .await;
        }
    }

    fn clear_known_rooms(&self) {
        if let Ok(mut known_room_ids) = self.known_room_ids.write() {
            known_room_ids.clear();
        }
    }

    fn ensure_known_room_for_message_interaction(
        &self,
        request_id: RequestId,
        room_id: &str,
    ) -> bool {
        let known = self
            .known_room_ids
            .read()
            .map(|known_room_ids| known_room_ids.contains(room_id))
            .unwrap_or(false);
        if !known {
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::NotFound,
                },
            );
        }
        known
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

    fn reduce(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.try_send(actions);
    }
}

// ---------------------------------------------------------------------------
// Room list refresh + observation loop
// ---------------------------------------------------------------------------

/// Maximum number of room-list entries requested from the live service's
/// dynamic entries adapter (mirrors the auth snapshot limit).
const ROOM_LIST_ENTRIES_LIMIT: usize = 4096;

/// Normalize a snapshot and project it as `AppAction::RoomListUpdated` +
/// `RoomEvent::RoomListUpdated`.
fn project_room_list_snapshot(
    snapshot: &matrix_desktop_sdk::MatrixRoomListSnapshot,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) {
    let spaces = normalize_spaces(snapshot);
    let rooms = normalize_rooms(snapshot);
    let invites = normalize_invites(snapshot);
    replace_known_room_ids(known_room_ids, &rooms);
    let _ = action_tx.try_send(vec![
        AppAction::RoomListUpdated { spaces, rooms },
        AppAction::InviteListUpdated { invites },
    ]);
    let _ = event_tx.send(CoreEvent::Room(RoomEvent::RoomListUpdated));
}

/// LegacySync-path refresh: normalize from `client.joined_rooms()` and
/// project. Never constructs a `RoomListService` (canon prohibition).
async fn refresh_room_list_from_joined_rooms(
    session: &MatrixClientSession,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) {
    let snapshot = matrix_desktop_sdk::room_list_snapshot_from_sdk_rooms_with_invites(
        session,
        session.client().joined_rooms(),
    )
    .await;
    project_room_list_snapshot(&snapshot, known_room_ids, action_tx, event_tx);
}

/// SyncService-path observation loop (Async rule 1: relay the SDK's
/// observable streams). Subscribes to the live `RoomListService`'s
/// `all_rooms()` entries stream (`entries_with_dynamic_adapters` with the
/// non-left filter — the same shape the live service drives with its
/// `required_state`, including `m.room.create` for space classification) and
/// KEEPS CONSUMING it: the current entry vector is maintained by applying
/// each `VectorDiff` batch, and every joined/invited batch triggers a
/// re-normalization.
/// The first batch (a Reset with the current entries) doubles as the initial
/// snapshot. A refresh request (operation-triggered) re-normalizes from the
/// current entries without touching the service. Exits on the oneshot stop
/// signal or when the stream ends.
async fn run_live_room_list_observation(
    session: Arc<MatrixClientSession>,
    service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut refresh_rx: mpsc::Receiver<()>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    use futures_util::StreamExt as _;

    let Ok(all_rooms) = service.all_rooms().await else {
        return;
    };
    let (entries, entries_controller) =
        all_rooms.entries_with_dynamic_adapters(ROOM_LIST_ENTRIES_LIMIT);
    entries_controller.set_filter(Box::new(
        matrix_sdk_ui::room_list_service::filters::new_filter_non_left(),
    ));
    let mut entries = Box::pin(entries);

    // Current filtered entry vector, maintained by applying each diff batch.
    let mut current: eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem> =
        eyeball_im::Vector::new();

    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            _ = refresh_rx.recv() => {
                // Operation-triggered refresh: drain coalesced requests, then
                // re-normalize from the live service's CURRENT entries.
                while refresh_rx.try_recv().is_ok() {}
                normalize_and_project_entries(
                    &session,
                    &current,
                    &known_room_ids,
                    &action_tx,
                    &event_tx,
                ).await;
            }
            maybe_diffs = entries.next() => match maybe_diffs {
                None => break,
                Some(diffs) => {
                    for diff in diffs {
                        diff.apply(&mut current);
                    }
                    normalize_and_project_entries(
                        &session,
                        &current,
                        &known_room_ids,
                        &action_tx,
                        &event_tx,
                    ).await;
                }
            },
        }
    }
}

/// Normalize the live service's current entries and project the result.
async fn normalize_and_project_entries(
    session: &MatrixClientSession,
    current: &eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) {
    // Collect before the await: mapping lazily across the await trips a
    // higher-ranked lifetime check on the iterator closure.
    let mut rooms = Vec::with_capacity(current.len());
    for item in current.iter() {
        rooms.push(item.clone().into_inner());
    }
    let snapshot =
        matrix_desktop_sdk::room_list_snapshot_from_sdk_rooms_with_invites(session, rooms).await;
    project_room_list_snapshot(&snapshot, known_room_ids, action_tx, event_tx);
}

/// LegacySync-path observation loop (Async rule 1: relay the SDK's observable
/// streams). Subscribes to `client.subscribe_to_all_room_updates()`, which
/// fires on the legacy backend because it feeds the base client. Each
/// received batch coalesces any additionally pending batches into one
/// re-normalization; `Lagged` triggers a single refresh because the snapshot
/// is self-healing. Exits on the oneshot stop signal (same pattern as
/// `sync.rs` `legacy_stop_tx`) or when the SDK closes the broadcast.
async fn run_legacy_room_list_observation(
    session: Arc<MatrixClientSession>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    use tokio::sync::broadcast::error::RecvError;

    let mut updates_rx = session.client().subscribe_to_all_room_updates();
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            result = updates_rx.recv() => match result {
                Ok(_batch) => {
                    // Coalesce: drain any additionally pending update batches;
                    // one refresh covers them all.
                    while updates_rx.try_recv().is_ok() {}
                    refresh_room_list_from_joined_rooms(
                        &session,
                        &known_room_ids,
                        &action_tx,
                        &event_tx,
                    ).await;
                }
                Err(RecvError::Lagged(_)) => {
                    // The snapshot is self-healing: refresh once.
                    refresh_room_list_from_joined_rooms(
                        &session,
                        &known_room_ids,
                        &action_tx,
                        &event_tx,
                    ).await;
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Normalization helpers: auth snapshot → state DTOs
// ---------------------------------------------------------------------------

/// Convert `MatrixRoomListSnapshot` spaces into `SpaceSummary` values with
/// child room id lists. child_room_ids is populated by cross-referencing the
/// rooms' `parent_space_ids`.
fn normalize_spaces(snapshot: &matrix_desktop_sdk::MatrixRoomListSnapshot) -> Vec<SpaceSummary> {
    snapshot
        .spaces
        .iter()
        .map(|space| {
            let child_room_ids: Vec<String> = snapshot
                .rooms
                .iter()
                .filter(|room| room.parent_space_ids.iter().any(|id| id == &space.space_id))
                .map(|room| room.room_id.clone())
                .collect();
            SpaceSummary {
                space_id: space.space_id.clone(),
                display_name: space.display_name.clone(),
                avatar: avatar_from_mxc_uri(space.avatar_mxc_uri.as_deref()),
                child_room_ids,
            }
        })
        .collect()
}

/// Convert `MatrixRoomListSnapshot` rooms into `RoomSummary` values.
fn normalize_rooms(snapshot: &matrix_desktop_sdk::MatrixRoomListSnapshot) -> Vec<RoomSummary> {
    snapshot
        .rooms
        .iter()
        .map(|room| RoomSummary {
            room_id: room.room_id.clone(),
            display_name: room.display_name.clone(),
            avatar: avatar_from_mxc_uri(room.avatar_mxc_uri.as_deref()),
            is_dm: room.is_dm,
            tags: normalize_room_tags(&room.tags),
            unread_count: room.unread_count,
            notification_count: room.notification_count,
            highlight_count: room.highlight_count,
            parent_space_ids: room.parent_space_ids.clone(),
        })
        .collect()
}

fn normalize_room_tags(tags: &MatrixRoomTags) -> RoomTags {
    RoomTags {
        favourite: tags.favourite.as_ref().map(|info| RoomTagInfo {
            order: info.order.clone(),
        }),
        low_priority: tags.low_priority.as_ref().map(|info| RoomTagInfo {
            order: info.order.clone(),
        }),
    }
}

fn pinned_events_from_ids(event_ids: Vec<String>) -> Vec<PinnedEvent> {
    event_ids
        .into_iter()
        .map(|event_id| PinnedEvent {
            event_id,
            sender: None,
            body_preview: None,
            redacted: false,
        })
        .collect()
}

fn replace_known_room_ids(known_room_ids: &Arc<RwLock<BTreeSet<String>>>, rooms: &[RoomSummary]) {
    if let Ok(mut known_room_ids) = known_room_ids.write() {
        *known_room_ids = rooms.iter().map(|room| room.room_id.clone()).collect();
    }
}

/// Convert `MatrixRoomListSnapshot` invites into Rust-owned invite previews.
fn normalize_invites(snapshot: &matrix_desktop_sdk::MatrixRoomListSnapshot) -> Vec<InvitePreview> {
    snapshot
        .invites
        .iter()
        .map(|invite| InvitePreview {
            room_id: invite.room_id.clone(),
            display_name: invite.display_name.clone(),
            avatar: avatar_from_mxc_uri(invite.avatar_mxc_uri.as_deref()),
            topic: invite.topic.clone(),
            inviter_display_name: invite.inviter_display_name.clone(),
            is_dm: invite.is_dm,
        })
        .collect()
}

fn directory_room_summary_from_sdk(room: MatrixPublicRoomDirectoryRoom) -> DirectoryRoomSummary {
    DirectoryRoomSummary {
        room_id: room.room_id,
        canonical_alias: room.canonical_alias,
        name: room.name,
        topic: room.topic,
        avatar_url: room.avatar_url,
        joined_members: room.joined_members,
        world_readable: room.world_readable,
        guest_can_join: room.guest_can_join,
    }
}

fn avatar_from_mxc_uri(mxc_uri: Option<&str>) -> Option<AvatarImage> {
    mxc_uri.map(|mxc_uri| AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    })
}

fn sdk_room_tag_kind(tag: RoomTagKind) -> MatrixRoomTagKind {
    match tag {
        RoomTagKind::Favourite => MatrixRoomTagKind::Favourite,
        RoomTagKind::LowPriority => MatrixRoomTagKind::LowPriority,
    }
}

fn room_tag_info_from_order(order: Option<f64>) -> RoomTagInfo {
    RoomTagInfo {
        order: order.map(|order| order.to_string()),
    }
}

fn operation_failure_kind(kind: RoomFailureKind) -> OperationFailureKind {
    match kind {
        RoomFailureKind::Forbidden => OperationFailureKind::Forbidden,
        RoomFailureKind::Network => OperationFailureKind::Network,
        RoomFailureKind::NotFound => OperationFailureKind::NotFound,
        RoomFailureKind::Sdk => OperationFailureKind::Sdk,
    }
}

// ---------------------------------------------------------------------------
// Error classification (never raw SDK text in public events)
// ---------------------------------------------------------------------------

/// Map a `MatrixRoomOperationError` to a coarse `RoomFailureKind`.
/// The spec defines: Forbidden / NotFound / Network / Sdk.
/// Raw SDK error text must never appear in public events.
pub(crate) fn classify_room_error(error: &MatrixRoomOperationError) -> RoomFailureKind {
    use matrix_desktop_sdk::MatrixRoomOperationFailureKind;
    match error {
        MatrixRoomOperationError::InvalidRoomId
        | MatrixRoomOperationError::InvalidRoomAlias
        | MatrixRoomOperationError::InvalidEventId
        | MatrixRoomOperationError::InvalidUserId
        | MatrixRoomOperationError::InvalidServerName
        | MatrixRoomOperationError::RoomUnavailable => RoomFailureKind::NotFound,
        MatrixRoomOperationError::Sdk(kind) => match kind {
            MatrixRoomOperationFailureKind::Forbidden
            | MatrixRoomOperationFailureKind::AuthenticationRequired => RoomFailureKind::Forbidden,
            MatrixRoomOperationFailureKind::Http => RoomFailureKind::Network,
            MatrixRoomOperationFailureKind::Sdk
            | MatrixRoomOperationFailureKind::Encryption
            | MatrixRoomOperationFailureKind::Store
            | MatrixRoomOperationFailureKind::WrongRoomState => RoomFailureKind::Sdk,
        },
    }
}

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use matrix_desktop_sdk::{
        MatrixInvitePreview, MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace,
        MatrixRoomTagInfo, MatrixRoomTags,
    };
    use matrix_desktop_state::{RoomTagInfo, RoomTagKind};
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::command::RoomCommand;
    use crate::event::CoreEvent;
    use crate::failure::{CoreFailure, RoomFailureKind};
    use crate::ids::{RequestId, RuntimeConnectionId};

    fn make_request_id(seq: u64) -> RequestId {
        RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: seq,
        }
    }

    // --- Error classification ---

    #[test]
    fn forbidden_sdk_error_classifies_as_forbidden() {
        let error = MatrixRoomOperationError::Sdk(
            matrix_desktop_sdk::MatrixRoomOperationFailureKind::Forbidden,
        );
        assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
    }

    #[test]
    fn auth_required_sdk_error_classifies_as_forbidden() {
        let error = MatrixRoomOperationError::Sdk(
            matrix_desktop_sdk::MatrixRoomOperationFailureKind::AuthenticationRequired,
        );
        assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
    }

    #[test]
    fn http_sdk_error_classifies_as_network() {
        let error =
            MatrixRoomOperationError::Sdk(matrix_desktop_sdk::MatrixRoomOperationFailureKind::Http);
        assert_eq!(classify_room_error(&error), RoomFailureKind::Network);
    }

    #[test]
    fn invalid_room_id_classifies_as_not_found() {
        let error = MatrixRoomOperationError::InvalidRoomId;
        assert_eq!(classify_room_error(&error), RoomFailureKind::NotFound);
    }

    #[test]
    fn room_unavailable_classifies_as_not_found() {
        let error = MatrixRoomOperationError::RoomUnavailable;
        assert_eq!(classify_room_error(&error), RoomFailureKind::NotFound);
    }

    #[test]
    fn sdk_error_classifies_as_sdk() {
        let error =
            MatrixRoomOperationError::Sdk(matrix_desktop_sdk::MatrixRoomOperationFailureKind::Sdk);
        assert_eq!(classify_room_error(&error), RoomFailureKind::Sdk);
    }

    // --- Room list normalization: spaces ---

    #[test]
    fn normalize_spaces_with_child_rooms() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space1:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
            }],
            rooms: vec![
                MatrixRoomListRoom {
                    room_id: "!room1:example.test".to_owned(),
                    display_name: "Room 1".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: false,
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    parent_space_ids: vec!["!space1:example.test".to_owned()],
                },
                MatrixRoomListRoom {
                    room_id: "!room2:example.test".to_owned(),
                    display_name: "Room 2".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: false,
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    parent_space_ids: vec![],
                },
            ],
            ..MatrixRoomListSnapshot::default()
        };
        let spaces = normalize_spaces(&snapshot);
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].space_id, "!space1:example.test");
        assert_eq!(spaces[0].child_room_ids, vec!["!room1:example.test"]);
    }

    #[test]
    fn normalize_spaces_no_children() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Empty Space".to_owned(),
                avatar_mxc_uri: None,
            }],
            rooms: vec![],
            ..MatrixRoomListSnapshot::default()
        };
        let spaces = normalize_spaces(&snapshot);
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].child_room_ids, Vec::<String>::new());
    }

    #[test]
    fn normalize_spaces_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Space".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/space-avatar".to_owned()),
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let spaces = normalize_spaces(&snapshot);

        let avatar = spaces[0].avatar.as_ref().expect("space avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/space-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    // --- Room list normalization: rooms ---

    #[test]
    fn normalize_rooms_preserves_dm_and_unread() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!dm:example.test".to_owned(),
                display_name: "Alice".to_owned(),
                avatar_mxc_uri: None,
                is_dm: true,
                tags: MatrixRoomTags::default(),
                unread_count: 3,
                notification_count: 3,
                highlight_count: 1,
                parent_space_ids: vec![],
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_id, "!dm:example.test");
        assert!(rooms[0].is_dm);
        assert_eq!(rooms[0].unread_count, 3);
        assert_eq!(rooms[0].notification_count, 3);
        assert_eq!(rooms[0].highlight_count, 1);
    }

    #[test]
    fn normalize_rooms_non_dm() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                parent_space_ids: vec!["!space:example.test".to_owned()],
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);
        assert_eq!(rooms.len(), 1);
        assert!(!rooms[0].is_dm);
        assert_eq!(rooms[0].parent_space_ids, vec!["!space:example.test"]);
        assert_eq!(rooms[0].notification_count, 0);
        assert_eq!(rooms[0].highlight_count, 0);
    }

    #[test]
    fn normalize_rooms_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/room-avatar".to_owned()),
                is_dm: false,
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                parent_space_ids: vec![],
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);

        let avatar = rooms[0].avatar.as_ref().expect("room avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/room-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn normalize_invites_preserves_preview_fields() {
        let snapshot = MatrixRoomListSnapshot {
            invites: vec![MatrixInvitePreview {
                room_id: "!invite:example.test".to_owned(),
                display_name: "Project invite".to_owned(),
                avatar_mxc_uri: None,
                topic: Some("Project topic".to_owned()),
                inviter_display_name: Some("Inviter".to_owned()),
                is_dm: true,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let invites = normalize_invites(&snapshot);

        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].room_id, "!invite:example.test");
        assert_eq!(invites[0].display_name, "Project invite");
        assert_eq!(invites[0].topic.as_deref(), Some("Project topic"));
        assert_eq!(invites[0].inviter_display_name.as_deref(), Some("Inviter"));
        assert!(invites[0].is_dm);
    }

    #[test]
    fn normalize_invites_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            invites: vec![MatrixInvitePreview {
                room_id: "!invite:example.test".to_owned(),
                display_name: "Invite".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/invite-avatar".to_owned()),
                topic: None,
                inviter_display_name: None,
                is_dm: false,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let invites = normalize_invites(&snapshot);

        let avatar = invites[0].avatar.as_ref().expect("invite avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/invite-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    // --- SelectSpace / SelectRoom projection ---

    #[tokio::test]
    async fn select_space_projects_action() {
        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        handle
            .send(RoomMessage::Command(RoomCommand::SelectSpace {
                request_id: make_request_id(1),
                space_id: Some("!space:example.test".to_owned()),
            }))
            .await;

        let actions = action_rx.recv().await.expect("actions");
        assert!(
            matches!(
                actions.as_slice(),
                [AppAction::SelectSpace {
                    space_id: Some(id)
                }] if id == "!space:example.test"
            ),
            "expected SelectSpace action, got {actions:?}"
        );
    }

    #[test]
    fn normalize_rooms_carries_sdk_room_tags() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room1:example.test".to_owned(),
                display_name: "Room 1".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                tags: MatrixRoomTags {
                    favourite: Some(MatrixRoomTagInfo {
                        order: Some("0.25".to_owned()),
                    }),
                    low_priority: None,
                },
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                parent_space_ids: vec![],
            }],
            invites: vec![],
        };

        let rooms = normalize_rooms(&snapshot);

        assert_eq!(
            rooms[0].tags.favourite,
            Some(RoomTagInfo {
                order: Some("0.25".to_owned())
            })
        );
        assert_eq!(rooms[0].tags.low_priority, None);
    }

    #[tokio::test]
    async fn select_room_projects_action() {
        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        handle
            .send(RoomMessage::Command(RoomCommand::SelectRoom {
                request_id: make_request_id(2),
                room_id: "!room:example.test".to_owned(),
            }))
            .await;

        let actions = action_rx.recv().await.expect("actions");
        assert!(
            matches!(
                actions.as_slice(),
                [AppAction::SelectRoom { room_id }] if room_id == "!room:example.test"
            ),
            "expected SelectRoom action, got {actions:?}"
        );
    }

    // --- OperationFailed without session emits SessionRequired ---

    #[tokio::test]
    async fn create_room_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(3);
        handle
            .send(RoomMessage::Command(RoomCommand::CreateRoom {
                request_id,
                name: "test room".to_owned(),
                encrypted: false,
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn leave_room_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(4);
        handle
            .send(RoomMessage::Command(RoomCommand::LeaveRoom {
                request_id,
                room_id: "!room:example.test".to_owned(),
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_room_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(5);
        handle
            .send(RoomMessage::Command(RoomCommand::ForgetRoom {
                request_id,
                room_id: "!room:example.test".to_owned(),
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_room_tag_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(6);
        handle
            .send(RoomMessage::Command(RoomCommand::SetTag {
                request_id,
                room_id: "!room:example.test".to_owned(),
                tag: RoomTagKind::Favourite,
                order: None,
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_room_tag_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(7);
        handle
            .send(RoomMessage::Command(RoomCommand::RemoveTag {
                request_id,
                room_id: "!room:example.test".to_owned(),
                tag: RoomTagKind::LowPriority,
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pin_event_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(8);
        handle
            .send(RoomMessage::Command(RoomCommand::PinEvent {
                request_id,
                room_id: "!room:example.test".to_owned(),
                event_id: "$event:example.test".to_owned(),
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unpin_event_without_session_emits_session_required() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        let request_id = make_request_id(9);
        handle
            .send(RoomMessage::Command(RoomCommand::UnpinEvent {
                request_id,
                room_id: "!room:example.test".to_owned(),
                event_id: "$event:example.test".to_owned(),
            }))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("timeout")
            .expect("event");

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    #[test]
    fn room_tag_success_path_does_not_refresh_from_stale_sdk_snapshot() {
        let source = include_str!("room.rs");
        let set_tag_body = source
            .split("async fn handle_set_tag")
            .nth(1)
            .expect("set tag handler")
            .split("async fn handle_remove_tag")
            .next()
            .expect("set tag body");
        let remove_tag_body = source
            .split("async fn handle_remove_tag")
            .nth(1)
            .expect("remove tag handler")
            .split("    /// Refresh the room list")
            .next()
            .expect("remove tag body");

        assert!(!set_tag_body.contains("refresh_room_list().await"));
        assert!(!remove_tag_body.contains("refresh_room_list().await"));
    }

    #[test]
    fn pin_success_settles_pending_before_pinned_projection_reload() {
        let source = include_str!("room.rs");
        let pin_body = source
            .split("async fn handle_pin_event")
            .nth(1)
            .expect("pin handler")
            .split("async fn handle_unpin_event")
            .next()
            .expect("pin body");
        let unpin_body = source
            .split("async fn handle_unpin_event")
            .nth(1)
            .expect("unpin handler")
            .split("async fn project_pinned_events_after_success")
            .next()
            .expect("unpin body");
        let projection_body = source
            .split("async fn project_pinned_events_after_success")
            .nth(1)
            .expect("projection helper")
            .split("    /// Refresh the room list")
            .next()
            .expect("projection body");

        let pin_completion = pin_body
            .find("self.reduce(vec![AppAction::PinEventCompleted")
            .expect("pin completion action");
        let pin_reload = pin_body
            .find("project_pinned_events_after_success")
            .expect("pin projection reload");
        assert!(pin_completion < pin_reload);

        let unpin_completion = unpin_body
            .find("self.reduce(vec![AppAction::UnpinEventCompleted")
            .expect("unpin completion action");
        let unpin_reload = unpin_body
            .find("project_pinned_events_after_success")
            .expect("unpin projection reload");
        assert!(unpin_completion < unpin_reload);

        assert!(!projection_body.contains("AppAction::PinEventCompleted"));
        assert!(!projection_body.contains("AppAction::UnpinEventCompleted"));
    }

    #[test]
    fn pin_and_unpin_commands_require_actor_known_room_guard_before_sdk_call() {
        let source = include_str!("room.rs");
        let pin_body = source
            .split("async fn handle_pin_event")
            .nth(1)
            .expect("pin handler")
            .split("async fn handle_unpin_event")
            .next()
            .expect("pin body");
        let unpin_body = source
            .split("async fn handle_unpin_event")
            .nth(1)
            .expect("unpin handler")
            .split("async fn project_pinned_events_after_success")
            .next()
            .expect("unpin body");

        let pin_guard = pin_body
            .find("ensure_known_room_for_message_interaction")
            .expect("pin known-room guard");
        let pin_sdk = pin_body
            .find("matrix_desktop_sdk::pin_event")
            .expect("pin sdk call");
        assert!(pin_guard < pin_sdk);

        let unpin_guard = unpin_body
            .find("ensure_known_room_for_message_interaction")
            .expect("unpin known-room guard");
        let unpin_sdk = unpin_body
            .find("matrix_desktop_sdk::unpin_event")
            .expect("unpin sdk call");
        assert!(unpin_guard < unpin_sdk);
    }

    // --- request_id correlation on RoomEvents ---

    #[test]
    fn room_event_carries_request_id() {
        let request_id = make_request_id(10);
        let event = RoomEvent::RoomCreated {
            request_id,
            room_id: "!room:example.test".to_owned(),
        };
        match event {
            RoomEvent::RoomCreated {
                request_id: ev_id, ..
            } => assert_eq!(ev_id, request_id),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    // --- Observation lifecycle messages without a session are safe ---

    #[tokio::test]
    async fn sync_stopped_and_shutdown_without_session_complete_cleanly() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        // No session, no observation loop: both must be no-ops, and the
        // actor task must still exit on Shutdown.
        assert!(handle.send(RoomMessage::SyncStopped).await);
        assert!(handle.send(RoomMessage::Shutdown).await);
        tokio::time::timeout(std::time::Duration::from_secs(5), handle.join())
            .await
            .expect("actor task must exit after Shutdown");
    }

    // --- Normalization empty snapshot ---

    #[test]
    fn normalize_empty_snapshot() {
        let snapshot = MatrixRoomListSnapshot::default();
        assert!(normalize_spaces(&snapshot).is_empty());
        assert!(normalize_rooms(&snapshot).is_empty());
    }
}
