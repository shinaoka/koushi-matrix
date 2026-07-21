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
//! `LeaveRoom`, and `ForgetRoom` call `koushi-sdk` primitives and emit
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

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::{
    MatrixClientSession, MatrixCreateRoomOptions, MatrixCreateRoomParentSpace,
    MatrixCreateRoomVisibility, MatrixPublicRoomDirectoryQuery, MatrixPublicRoomDirectoryRoom,
    MatrixRoomHistoryVisibility, MatrixRoomJoinRule, MatrixRoomListRoom, MatrixRoomListSnapshot,
    MatrixRoomListSpace, MatrixRoomMemberRole, MatrixRoomMemberSummary, MatrixRoomModerationAction,
    MatrixRoomOperationError, MatrixRoomPermissionFacts, MatrixRoomSettingChange,
    MatrixRoomSettingsSnapshot, MatrixRoomTagKind, MatrixRoomTags, MatrixUserTrustState,
};
use koushi_state::{
    AppAction, AvatarImage, AvatarThumbnailState, BasicOperationRequest, DirectoryQuery,
    DirectoryRoomSummary, INVITE_ALREADY_IN_SPACE_MESSAGE, InviteDestination,
    InviteDestinationResult, InviteDestinationResultKind, InvitePreview, InviteScopeSelection,
    OperationFailureKind, PinnedEvent, RoomHistoryVisibility, RoomJoinRule, RoomMemberRole,
    RoomMemberSummary, RoomModerationAction, RoomNotificationMode, RoomPermissionFacts,
    RoomSettingChange, RoomSettingsSnapshot, RoomSummary, RoomTagInfo, RoomTagKind, RoomTags,
    SpaceSummary, UserProfile, UserTrustState,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::command::{CreateRoomOptions, CreateRoomVisibility, RoomCommand};
use crate::event::{CoreEvent, ReportKind, RoomEvent};
use crate::executor;
use crate::failure::{CoreFailure, RoomFailureKind};
use crate::ids::RequestId;
use crate::unread_trace;

/// Fixed, content-free messages recorded in `AppState.errors` when a basic
/// operation fails. Raw SDK errors are classified into `RoomFailureKind` for the
/// transport `OperationFailed` event and never placed in product state.
const CREATE_ROOM_FAILED_MESSAGE: &str = "Room creation failed";
const CREATE_SPACE_FAILED_MESSAGE: &str = "Space creation failed";
const LINK_SPACE_CHILD_FAILED_MESSAGE: &str = "Linking the room to the space failed";

type SpaceChildLinkKey = (String, String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingSpaceChildLink {
    space_id: String,
    child_room_id: String,
    via_server: String,
}

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
    /// The active account is logging out/switching/resetting while the
    /// RoomActor stays alive for future sessions.
    SessionCleared,
    /// Observer relay: parent-only space links discovered in a room-list
    /// snapshot. RoomActor owns dedupe, server writes, and retry policy.
    MissingSpaceChildLinks { links: Vec<MissingSpaceChildLink> },
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

    /// Wait for the actor task to complete (used in ordered shutdown).
    pub async fn join(self) {
        let _ = self.task.await;
    }
}

/// Handle on the spawned room-list observation loop: oneshot stop signal plus
/// the task handle so teardown can await completion (same pattern as
/// `sync.rs` `legacy_stop_tx`). Operation-triggered refreshes are always sent
/// to the observation loop so command handling never blocks on room-list
/// normalization.
struct RoomListObservation {
    stop_tx: oneshot::Sender<()>,
    task: executor::JoinHandle<()>,
    refresh_tx: mpsc::Sender<()>,
}

pub struct RoomActor {
    session: Option<Arc<MatrixClientSession>>,
    observation: Option<RoomListObservation>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    attempted_space_child_repairs: Arc<RwLock<BTreeSet<SpaceChildLinkKey>>>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    self_tx: mpsc::Sender<RoomMessage>,
    command_rx: mpsc::Receiver<RoomMessage>,
}

impl RoomActor {
    pub fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
    ) -> RoomActorHandle {
        let (tx, command_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let actor = RoomActor {
            session: None,
            observation: None,
            known_room_ids: Arc::new(RwLock::new(BTreeSet::new())),
            attempted_space_child_repairs: Arc::new(RwLock::new(BTreeSet::new())),
            action_tx,
            event_tx,
            self_tx: tx.clone(),
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
                    self.clear_space_child_repair_attempts();
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
                    self.clear_space_child_repair_attempts();
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
                            // LegacySync backend: relay the base client's
                            // room update broadcast (Async rule 1). Request
                            // the initial snapshot through the observation
                            // loop so SyncStarted never blocks this actor.
                            self.start_legacy_observation();
                            self.refresh_room_list();
                        }
                    }
                }
                RoomMessage::SyncStopped => {
                    self.stop_observation().await;
                    self.clear_known_rooms();
                    self.clear_space_child_repair_attempts();
                }
                RoomMessage::SessionCleared => {
                    self.stop_observation().await;
                    self.session = None;
                    self.clear_known_rooms();
                    self.clear_space_child_repair_attempts();
                }
                RoomMessage::MissingSpaceChildLinks { links } => {
                    self.handle_missing_space_child_links(links).await;
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
            self.self_tx.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            refresh_rx,
            stop_rx,
        ));
        self.observation = Some(RoomListObservation {
            stop_tx,
            task,
            refresh_tx,
        });
    }

    /// Spawn the legacy room-list observation loop (LegacySync backend) for
    /// the current session.
    fn start_legacy_observation(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (refresh_tx, refresh_rx) = mpsc::channel::<()>(8);
        let task = executor::spawn(run_legacy_room_list_observation(
            session.clone(),
            self.known_room_ids.clone(),
            self.self_tx.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            refresh_rx,
            stop_rx,
        ));
        self.observation = Some(RoomListObservation {
            stop_tx,
            task,
            refresh_tx,
        });
    }

    async fn handle_missing_space_child_links(&mut self, links: Vec<MissingSpaceChildLink>) {
        let Some(session) = self.session.clone() else {
            return;
        };

        for link in links {
            let key = (link.space_id.clone(), link.child_room_id.clone());
            let already_repaired = self
                .attempted_space_child_repairs
                .read()
                .map(|attempts| attempts.contains(&key))
                .unwrap_or(true);
            if already_repaired {
                continue;
            }

            match koushi_sdk::set_space_child(
                &session,
                &link.space_id,
                &link.child_room_id,
                &link.via_server,
            )
            .await
            {
                Ok(()) => {
                    if let Ok(mut attempts) = self.attempted_space_child_repairs.write() {
                        attempts.insert(key);
                    }
                    self.refresh_room_list();
                }
                Err(error) => {
                    let _kind = classify_room_error(&error);
                }
            }
        }
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
                options,
            } => {
                self.handle_create_room(request_id, options).await;
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
            RoomCommand::InviteTargets {
                request_id,
                room_id,
                user_ids,
                scope,
            } => {
                self.handle_invite_targets(request_id, room_id, user_ids, scope)
                    .await;
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
            RoomCommand::LoadRoomSettings {
                request_id,
                room_id,
            } => {
                self.handle_load_room_settings(request_id, room_id).await;
            }
            RoomCommand::ReshareRoomKey {
                request_id,
                room_id,
            } => {
                self.handle_reshare_room_key(request_id, room_id).await;
            }
            RoomCommand::UpdateRoomSetting {
                request_id,
                room_id,
                change,
            } => {
                self.handle_update_room_setting(request_id, room_id, change)
                    .await;
            }
            RoomCommand::ModerateRoomMember {
                request_id,
                room_id,
                target_user_id,
                action,
                reason,
            } => {
                self.handle_moderate_room_member(
                    request_id,
                    room_id,
                    target_user_id,
                    action,
                    reason,
                )
                .await;
            }
            RoomCommand::UpdateRoomMemberRole {
                request_id,
                room_id,
                target_user_id,
                power_level,
            } => {
                self.handle_update_room_member_role(
                    request_id,
                    room_id,
                    target_user_id,
                    power_level,
                )
                .await;
            }
            RoomCommand::SelectSpace {
                request_id: _,
                space_id,
            } => {
                // Pure navigation: project to reducer; no domain event.
                // request_id correlation via StateChanged is implicit per spec.
                // One-shot navigation MUST be delivered reliably (see reduce_reliable).
                self.reduce_reliable(vec![AppAction::SelectSpace { space_id }])
                    .await;
            }
            RoomCommand::ReorderSpaces {
                request_id: _,
                space_ids,
            } => {
                // Pure navigation preference: project to reducer; no domain event.
                // One-shot navigation MUST be delivered reliably (see reduce_reliable).
                self.reduce_reliable(vec![AppAction::ReorderSpaces { space_ids }])
                    .await;
            }
            RoomCommand::SelectRoom {
                request_id: _,
                room_id,
            } => {
                // Pure navigation: project to reducer; no domain event.
                // Core updates navigation state here and does not consume
                // reducer effects in this actor. One-shot navigation MUST be
                // delivered reliably: a dropped SelectRoom is the large-account
                // "room selection did not complete" bug (see reduce_reliable).
                self.reduce_reliable(vec![AppAction::SelectRoom { room_id }])
                    .await;
            }
            RoomCommand::MarkRoomAsRead {
                request_id,
                room_id,
                event_id,
            } => {
                self.handle_mark_room_as_read(request_id, room_id, event_id)
                    .await;
            }
            RoomCommand::MarkRoomAsUnread {
                request_id,
                room_id,
                unread,
            } => {
                self.handle_mark_room_as_unread(request_id, room_id, unread)
                    .await;
            }
            RoomCommand::SetRoomNotificationMode {
                request_id,
                room_id,
                mode,
            } => {
                self.handle_set_room_notification_mode(request_id, room_id, mode)
                    .await;
            }
            RoomCommand::ReportContent {
                request_id,
                room_id,
                event_id,
                reason,
            } => {
                self.handle_report_content(request_id, room_id, event_id, reason)
                    .await;
            }
            RoomCommand::ReportRoom {
                request_id,
                room_id,
                reason,
            } => {
                self.handle_report_room(request_id, room_id, reason).await;
            }
        }
    }

    async fn handle_create_room(&self, request_id: RequestId, options: CreateRoomOptions) {
        trace_room_operation("create_room", "start", request_id);
        let Some(session) = &self.session else {
            trace_room_operation("create_room", "session_required", request_id);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let name = options.name.clone();
        let parent_space = options.parent_space.clone();
        // Drive the basic-operation state machine: Idle -> CreatingRoom. The
        // reducer guards re-entry; `request_id.sequence` is the correlation id
        // the settle action below must match.
        self.reduce_reliable(vec![AppAction::BasicOperationRequested {
            request_id: request_id.sequence,
            request: BasicOperationRequest::CreateRoom { name: name.clone() },
        }])
        .await;
        match koushi_sdk::create_room(session, matrix_create_room_options(options)).await {
            Ok(room_id) => {
                trace_room_operation("create_room", "succeeded", request_id);
                self.link_created_room_to_parent_space(
                    session,
                    parent_space.as_ref(),
                    &room_id,
                    request_id,
                )
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomCreated {
                    request_id,
                    room_id,
                }));
                self.reduce_reliable(vec![AppAction::BasicOperationSucceeded {
                    request_id: request_id.sequence,
                }])
                .await;
                // Reflect the actor's own mutation immediately instead of
                // waiting for the next sync round-trip.
                self.refresh_room_list();
            }
            Err(error) => {
                trace_room_operation("create_room", "failed", request_id);
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                self.reduce_reliable(vec![AppAction::BasicOperationFailed {
                    request_id: request_id.sequence,
                    message: CREATE_ROOM_FAILED_MESSAGE.to_owned(),
                }])
                .await;
            }
        }
    }

    async fn link_created_room_to_parent_space(
        &self,
        session: &MatrixClientSession,
        parent_space: Option<&crate::command::CreateRoomParentSpace>,
        room_id: &str,
        request_id: RequestId,
    ) {
        let Some(parent_space) = parent_space else {
            return;
        };
        let Ok(via_server) = koushi_sdk::room_id_server_name(room_id) else {
            return;
        };

        match koushi_sdk::set_space_child(session, &parent_space.space_id, room_id, &via_server)
            .await
        {
            Ok(()) => {
                self.mark_space_child_link_attempted(&parent_space.space_id, room_id);
                self.emit(CoreEvent::Room(RoomEvent::SpaceChildSet {
                    request_id,
                    space_id: parent_space.space_id.clone(),
                    child_room_id: room_id.to_owned(),
                }));
            }
            Err(_) => {}
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

        match koushi_sdk::create_public_directory_room(session, &name, &alias_localpart).await {
            Ok(room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomCreated {
                    request_id,
                    room_id,
                }));
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_create_space(&self, request_id: RequestId, name: String) {
        trace_room_operation("create_space", "start", request_id);
        let Some(session) = &self.session else {
            trace_room_operation("create_space", "session_required", request_id);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        // Drive the basic-operation state machine: Idle -> CreatingSpace.
        self.reduce_reliable(vec![AppAction::BasicOperationRequested {
            request_id: request_id.sequence,
            request: BasicOperationRequest::CreateSpace { name: name.clone() },
        }])
        .await;
        match koushi_sdk::create_space(session, &name).await {
            Ok(space_id) => {
                trace_room_operation("create_space", "succeeded", request_id);
                self.emit(CoreEvent::Room(RoomEvent::SpaceCreated {
                    request_id,
                    space_id,
                }));
                self.reduce_reliable(vec![AppAction::BasicOperationSucceeded {
                    request_id: request_id.sequence,
                }])
                .await;
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list();
            }
            Err(error) => {
                trace_room_operation("create_space", "failed", request_id);
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                self.reduce_reliable(vec![AppAction::BasicOperationFailed {
                    request_id: request_id.sequence,
                    message: CREATE_SPACE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
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
        self.reduce_reliable(vec![AppAction::BasicOperationRequested {
            request_id: request_id.sequence,
            request: BasicOperationRequest::LinkSpaceChild {
                space_id: space_id.clone(),
                child_room_id: child_room_id.clone(),
            },
        }])
        .await;
        match koushi_sdk::set_space_child(session, &space_id, &child_room_id, &via_server).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::SpaceChildSet {
                    request_id,
                    space_id,
                    child_room_id,
                }));
                self.reduce_reliable(vec![AppAction::BasicOperationSucceeded {
                    request_id: request_id.sequence,
                }])
                .await;
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                self.reduce_reliable(vec![AppAction::BasicOperationFailed {
                    request_id: request_id.sequence,
                    message: LINK_SPACE_CHILD_FAILED_MESSAGE.to_owned(),
                }])
                .await;
            }
        }
    }

    async fn handle_invite_user(&self, request_id: RequestId, room_id: String, user_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match koushi_sdk::invite_user_to_room(session, &room_id, &user_id).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::UserInvited {
                    request_id,
                    room_id,
                    user_id,
                }));
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_invite_targets(
        &self,
        request_id: RequestId,
        room_id: String,
        user_ids: Vec<String>,
        scope: InviteScopeSelection,
    ) {
        self.reduce_reliable(vec![AppAction::InviteBatchRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            user_ids: user_ids.clone(),
            scope: scope.clone(),
        }])
        .await;

        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            self.reduce_reliable(vec![AppAction::InviteBatchFailed {
                request_id: request_id.sequence,
                room_id,
                kind: OperationFailureKind::Sdk,
            }])
            .await;
            return;
        };

        let mut results = Vec::new();
        let mut any_invited = false;

        for user_id in user_ids {
            if let InviteScopeSelection::ParentSpaceAndRoom { space_id } = &scope {
                match invite_target_to_space_if_needed(session, space_id, &user_id).await {
                    InviteTargetOutcome::Invited => {
                        any_invited = true;
                        results.push(InviteDestinationResult {
                            user_id: user_id.clone(),
                            destination: InviteDestination::Space {
                                space_id: space_id.clone(),
                            },
                            kind: InviteDestinationResultKind::Invited,
                            message: None,
                        });
                    }
                    InviteTargetOutcome::AlreadyInSpace => {
                        results.push(InviteDestinationResult {
                            user_id: user_id.clone(),
                            destination: InviteDestination::Space {
                                space_id: space_id.clone(),
                            },
                            kind: InviteDestinationResultKind::AlreadyInSpace,
                            message: Some(INVITE_ALREADY_IN_SPACE_MESSAGE.to_owned()),
                        });
                    }
                    InviteTargetOutcome::Failed => {
                        results.push(InviteDestinationResult {
                            user_id: user_id.clone(),
                            destination: InviteDestination::Space {
                                space_id: space_id.clone(),
                            },
                            kind: InviteDestinationResultKind::Failed,
                            message: None,
                        });
                    }
                }
            }

            match koushi_sdk::invite_user_to_room(session, &room_id, &user_id).await {
                Ok(()) => {
                    any_invited = true;
                    results.push(InviteDestinationResult {
                        user_id: user_id.clone(),
                        destination: InviteDestination::Room {
                            room_id: room_id.clone(),
                        },
                        kind: InviteDestinationResultKind::Invited,
                        message: None,
                    });
                }
                Err(_error) => {
                    results.push(InviteDestinationResult {
                        user_id,
                        destination: InviteDestination::Room {
                            room_id: room_id.clone(),
                        },
                        kind: InviteDestinationResultKind::Failed,
                        message: None,
                    });
                }
            }
        }

        self.reduce_reliable(vec![AppAction::InviteBatchCompleted {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            results: results.clone(),
        }])
        .await;
        self.emit(CoreEvent::Room(RoomEvent::InviteBatchCompleted {
            request_id,
            room_id,
            results,
        }));
        if any_invited {
            self.refresh_room_list();
        }
    }

    async fn handle_accept_invite(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match koushi_sdk::join_room_by_id(session, &room_id).await {
            Ok(joined_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::InviteAccepted {
                    request_id,
                    room_id: joined_room_id,
                }));
                self.refresh_room_list();
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
        match koushi_sdk::leave_room(session, &room_id).await {
            Ok(declined_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::InviteDeclined {
                    request_id,
                    room_id: declined_room_id,
                }));
                self.refresh_room_list();
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
        match koushi_sdk::start_direct_message(session, &user_id).await {
            Ok(room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::DirectMessageStarted {
                    request_id,
                    room_id,
                }));
                self.refresh_room_list();
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
        match koushi_sdk::join_room_by_id(session, &room_id).await {
            Ok(joined_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomJoined {
                    request_id,
                    room_id: joined_room_id,
                }));
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_query_directory(&self, request_id: RequestId, query: DirectoryQuery) {
        self.reduce_reliable(vec![AppAction::DirectoryQueryRequested {
            request_id: request_id.sequence,
            query: query.clone(),
        }])
        .await;
        let Some(session) = &self.session else {
            self.reduce_reliable(vec![AppAction::DirectoryQueryFailed {
                request_id: request_id.sequence,
                query,
                kind: OperationFailureKind::Sdk,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let sdk_query = MatrixPublicRoomDirectoryQuery {
            term: query.term.clone(),
            server_name: query.server_name.clone(),
            limit: query.limit,
            since: query.since.clone(),
        };
        match koushi_sdk::query_public_room_directory(session, sdk_query).await {
            Ok(result) => {
                let rooms: Vec<DirectoryRoomSummary> = result
                    .rooms
                    .into_iter()
                    .map(directory_room_summary_from_sdk)
                    .collect();
                self.reduce_reliable(vec![AppAction::DirectoryQuerySucceeded {
                    request_id: request_id.sequence,
                    query: query.clone(),
                    rooms: rooms.clone(),
                    next_batch: result.next_batch.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::DirectoryQueryCompleted {
                    request_id,
                    query,
                    rooms,
                    next_batch: result.next_batch,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::DirectoryQueryFailed {
                    request_id: request_id.sequence,
                    query,
                    kind: operation_failure_kind(kind),
                }])
                .await;
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
        self.reduce_reliable(vec![AppAction::DirectoryJoinRequested {
            request_id: request_id.sequence,
            alias: alias.clone(),
            via_server: via_server.clone(),
        }])
        .await;
        let Some(session) = &self.session else {
            self.reduce_reliable(vec![AppAction::DirectoryJoinFailed {
                request_id: request_id.sequence,
                alias,
                via_server,
                kind: OperationFailureKind::Sdk,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::join_room_by_alias(session, &alias, via_server.as_deref()).await {
            Ok(room_id) => {
                self.reduce_reliable(vec![AppAction::DirectoryJoinSucceeded {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomJoined {
                    request_id,
                    room_id,
                }));
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::DirectoryJoinFailed {
                    request_id: request_id.sequence,
                    alias,
                    via_server,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_load_room_settings(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => {
                let settings = room_settings_snapshot_from_sdk(settings);
                self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
                    room_id,
                    settings: settings.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomSettingsLoaded {
                    request_id,
                    settings,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_reshare_room_key(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::reshare_room_key(session, &room_id).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomKeyReshared {
                    request_id,
                    room_id,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_update_room_setting(
        &self,
        request_id: RequestId,
        room_id: String,
        change: RoomSettingChange,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let settings = match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => room_settings_snapshot_from_sdk(settings),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };
        self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.clone(),
            settings: settings.clone(),
        }])
        .await;
        if !settings.permissions.can_edit_settings {
            self.reduce_reliable(vec![AppAction::RoomSettingUpdateRequested {
                request_id: request_id.sequence,
                room_id,
                change,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::Forbidden,
                },
            );
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomSettingUpdateRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            change: change.clone(),
        }])
        .await;

        match koushi_sdk::update_room_setting(session, &room_id, room_setting_change_to_sdk(change))
            .await
        {
            Ok(settings) => {
                let settings = room_settings_snapshot_from_sdk(settings);
                self.reduce_reliable(vec![AppAction::RoomSettingUpdateSucceeded {
                    request_id: request_id.sequence,
                    room_id,
                    settings: settings.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                    request_id,
                    settings,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomSettingUpdateFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_moderate_room_member(
        &self,
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
        reason: Option<String>,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let settings = match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => room_settings_snapshot_from_sdk(settings),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };
        self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.clone(),
            settings: settings.clone(),
        }])
        .await;
        if !room_moderation_allowed(&settings.permissions, action) {
            self.reduce_reliable(vec![AppAction::RoomModerationRequested {
                request_id: request_id.sequence,
                room_id,
                target_user_id,
                action,
                reason,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::Forbidden,
                },
            );
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomModerationRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            target_user_id: target_user_id.clone(),
            action,
            reason: reason.clone(),
        }])
        .await;

        match koushi_sdk::moderate_room_member(
            session,
            &room_id,
            &target_user_id,
            room_moderation_action_to_sdk(action),
            reason.as_deref(),
        )
        .await
        {
            Ok(()) => {
                self.reduce_reliable(vec![AppAction::RoomModerationSucceeded {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                    target_user_id: target_user_id.clone(),
                    action,
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomMemberModerated {
                    request_id,
                    room_id,
                    target_user_id,
                    action,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomModerationFailed {
                    request_id: request_id.sequence,
                    room_id,
                    target_user_id,
                    action,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_update_room_member_role(
        &self,
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        power_level: i64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let settings = match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => room_settings_snapshot_from_sdk(settings),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };
        self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.clone(),
            settings: settings.clone(),
        }])
        .await;
        if !settings.permissions.can_edit_roles {
            self.reduce_reliable(vec![AppAction::RoomMemberRoleUpdateRequested {
                request_id: request_id.sequence,
                room_id,
                target_user_id,
                power_level,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::Forbidden,
                },
            );
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomMemberRoleUpdateRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            target_user_id: target_user_id.clone(),
            power_level,
        }])
        .await;

        match koushi_sdk::update_room_member_power_level(
            session,
            &room_id,
            &target_user_id,
            power_level,
        )
        .await
        {
            Ok(settings) => {
                let settings = room_settings_snapshot_from_sdk(settings);
                self.reduce_reliable(vec![
                    AppAction::RoomSettingsSnapshotLoaded {
                        room_id: room_id.clone(),
                        settings,
                    },
                    AppAction::RoomMemberRoleUpdateRequested {
                        request_id: request_id.sequence,
                        room_id: room_id.clone(),
                        target_user_id: target_user_id.clone(),
                        power_level,
                    },
                    AppAction::RoomMemberRoleUpdateSucceeded {
                        request_id: request_id.sequence,
                        room_id: room_id.clone(),
                        target_user_id: target_user_id.clone(),
                        power_level,
                    },
                ])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomMemberRoleUpdated {
                    request_id,
                    room_id,
                    target_user_id,
                    power_level,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomMemberRoleUpdateFailed {
                    request_id: request_id.sequence,
                    room_id,
                    target_user_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_leave_room(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match koushi_sdk::leave_room(session, &room_id).await {
            Ok(left_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomLeft {
                    request_id,
                    room_id: left_room_id,
                }));
                self.refresh_room_list();
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
        match koushi_sdk::forget_room(session, &room_id).await {
            Ok(forgotten_room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomForgotten {
                    request_id,
                    room_id: forgotten_room_id,
                }));
                self.refresh_room_list();
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
        match koushi_sdk::set_room_tag(session, &room_id, sdk_room_tag_kind(tag), order).await {
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
        match koushi_sdk::remove_room_tag(session, &room_id, sdk_room_tag_kind(tag)).await {
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

        self.reduce_reliable(vec![AppAction::PinEventRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        }])
        .await;
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match koushi_sdk::pin_event(session, &room_id, &event_id).await {
            Ok(()) => {
                self.reduce_reliable(vec![AppAction::PinEventCompleted {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::PinEventCompleted {
                    request_id,
                    room_id: room_id.clone(),
                }));
                self.project_pinned_events_after_success(request_id, room_id)
                    .await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::PinEventFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_unpin_event(&self, request_id: RequestId, room_id: String, event_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.reduce_reliable(vec![AppAction::UnpinEventRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        }])
        .await;
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match koushi_sdk::unpin_event(session, &room_id, &event_id).await {
            Ok(()) => {
                self.reduce_reliable(vec![AppAction::UnpinEventCompleted {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::UnpinEventCompleted {
                    request_id,
                    room_id: room_id.clone(),
                }));
                self.project_pinned_events_after_success(request_id, room_id)
                    .await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::UnpinEventFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn project_pinned_events_after_success(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            return;
        };
        let pinned = match koushi_sdk::load_pinned_event_ids(session, &room_id).await {
            Ok(event_ids) => pinned_events_from_ids(event_ids),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };

        self.reduce_reliable(vec![AppAction::RoomPinnedEventsUpdated {
            room_id: room_id.clone(),
            pinned: pinned.clone(),
        }])
        .await;
        self.emit(CoreEvent::Room(RoomEvent::PinnedEventsUpdated {
            room_id,
            pinned,
        }));
    }

    /// Request a room-list refresh and projection into AppState via the action
    /// channel. Also emits `RoomEvent::RoomListUpdated` as a discrete event.
    ///
    /// On the SyncService path this requests a re-normalization from the live
    /// service's current entries (inside the observation loop) — NEVER a new
    /// `RoomListService`. On the LegacySync path, the same request is handled
    /// by the legacy observation loop and coalesced there. Before sync starts,
    /// a detached one-shot refresh is spawned so room commands never await
    /// room-list normalization on the actor command loop.
    fn refresh_room_list(&self) {
        if let Some(observation) = &self.observation {
            let _ = observation.refresh_tx.try_send(());
            return;
        }
        if let Some(session) = self.session.clone() {
            let known_room_ids = self.known_room_ids.clone();
            let room_tx = self.self_tx.clone();
            let action_tx = self.action_tx.clone();
            let event_tx = self.event_tx.clone();
            let _ = executor::spawn(async move {
                refresh_room_list_from_joined_rooms(
                    &session,
                    &known_room_ids,
                    &room_tx,
                    &action_tx,
                    &event_tx,
                )
                .await;
            });
        }
    }

    async fn handle_mark_room_as_read(
        &self,
        request_id: RequestId,
        room_id: String,
        event_id: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        unread_trace::trace_mark_read(
            "mark_read_requested",
            request_id.sequence,
            &room_id,
            Some(event_id.as_str()),
        );
        self.reduce_reliable(vec![AppAction::RoomMarkedAsReadRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        }])
        .await;
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match koushi_sdk::mark_room_as_read(session, &room_id, &event_id).await {
            Ok(()) => {
                unread_trace::trace_mark_read(
                    "mark_read_success",
                    request_id.sequence,
                    &room_id,
                    Some(event_id.as_str()),
                );
                self.reduce_reliable(vec![
                    AppAction::FullyReadMarkerUpdated {
                        room_id: room_id.clone(),
                        event_id: Some(event_id.clone()),
                    },
                    AppAction::RoomMarkedAsReadSucceeded {
                        request_id: request_id.sequence,
                        room_id: room_id.clone(),
                    },
                ])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::MarkedAsRead {
                    request_id,
                    room_id: room_id.clone(),
                }));
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                unread_trace::trace_mark_read(
                    "mark_read_failed",
                    request_id.sequence,
                    &room_id,
                    Some(event_id.as_str()),
                );
                self.reduce_reliable(vec![AppAction::RoomMarkedAsReadFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_mark_room_as_unread(
        &self,
        request_id: RequestId,
        room_id: String,
        unread: bool,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.reduce_reliable(vec![AppAction::RoomMarkedAsUnreadRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            unread,
        }])
        .await;
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match koushi_sdk::mark_room_as_unread(session, &room_id, unread).await {
            Ok(()) => {
                self.reduce_reliable(vec![AppAction::RoomMarkedAsUnreadSucceeded {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                    unread,
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::MarkedAsUnread {
                    request_id,
                    room_id: room_id.clone(),
                    unread,
                }));
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomMarkedAsUnreadFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_set_room_notification_mode(
        &self,
        request_id: RequestId,
        room_id: String,
        mode: RoomNotificationMode,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomNotificationModeSet {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            mode,
        }])
        .await;
        match koushi_sdk::set_room_notification_mode(session, &room_id, mode).await {
            Ok(()) => {
                self.reduce_reliable(vec![AppAction::RoomNotificationModeCompleted {
                    request_id: request_id.sequence,
                    room_id,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomNotificationModeFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    async fn handle_report_content(
        &self,
        request_id: RequestId,
        room_id: String,
        event_id: String,
        reason: Option<String>,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::report_content(session, &room_id, &event_id, reason).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::ReportCompleted {
                    request_id,
                    kind: ReportKind::Event,
                }));
            }
            Err(error) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::ReportOperationFailed {
                        kind: classify_report_error(&error),
                    },
                );
            }
        }
    }

    async fn handle_report_room(&self, request_id: RequestId, room_id: String, reason: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::report_room(session, &room_id, reason).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::ReportCompleted {
                    request_id,
                    kind: ReportKind::Room,
                }));
            }
            Err(error) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::ReportOperationFailed {
                        kind: classify_report_error(&error),
                    },
                );
            }
        }
    }

    fn clear_known_rooms(&self) {
        if let Ok(mut known_room_ids) = self.known_room_ids.write() {
            known_room_ids.clear();
        }
    }

    fn clear_space_child_repair_attempts(&self) {
        if let Ok(mut attempts) = self.attempted_space_child_repairs.write() {
            attempts.clear();
        }
    }

    fn mark_space_child_link_attempted(&self, space_id: &str, child_room_id: &str) {
        if let Ok(mut attempts) = self.attempted_space_child_repairs.write() {
            attempts.insert((space_id.to_owned(), child_room_id.to_owned()));
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

    /// Reliable projection for one-shot, non-re-projected actions (navigation,
    /// command results) that MUST NOT be dropped under large-account sync load.
    /// Backpressures instead of dropping; the AppActor drains the action inbox
    /// continuously, so this does not deadlock.
    async fn reduce_reliable(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.send(actions).await;
    }
}

// ---------------------------------------------------------------------------
// Room list refresh + observation loop
// ---------------------------------------------------------------------------

/// Maximum number of room-list entries requested from the live service's
/// dynamic entries adapter (mirrors the auth snapshot limit).
const ROOM_LIST_ENTRIES_LIMIT: usize = 4096;

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum LiveObserverTestEvent {
    RlsProjected {
        wake_count: u64,
        entries_len: usize,
    },
    BaseBatch {
        wake_count: u64,
        update_count: u64,
        lagged: bool,
        projection_required: bool,
    },
    BaseProjected {
        wake_count: u64,
        rls_wake_count: u64,
        entries_len: usize,
        action_delivered: bool,
    },
    BaseClosed,
}

#[cfg(test)]
fn emit_live_observer_test_event(
    tx: &Option<mpsc::UnboundedSender<LiveObserverTestEvent>>,
    event: LiveObserverTestEvent,
) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}

/// Normalize a snapshot and project it as `AppAction::RoomListUpdated` +
/// `RoomEvent::RoomListUpdated`.
async fn project_room_list_snapshot(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) -> bool {
    let spaces = normalize_spaces(snapshot);
    let rooms = normalize_rooms(snapshot);
    let invites = normalize_invites(snapshot);
    let user_profiles = normalize_user_profiles(snapshot);
    unread_trace::trace_room_list_snapshot(&rooms);
    let projected_rooms = rooms.clone();
    let delivered = action_tx
        .send(vec![
            AppAction::RoomListUpdated { spaces, rooms },
            AppAction::InviteListUpdated { invites },
            AppAction::UserProfilesUpdated {
                profiles: user_profiles,
            },
        ])
        .await
        .is_ok();
    if delivered {
        replace_known_room_ids(known_room_ids, &projected_rooms);
        let _ = event_tx.send(CoreEvent::Room(RoomEvent::RoomListUpdated));
    }
    delivered
}

/// LegacySync-path refresh: normalize from `client.joined_rooms()` and
/// project. Never constructs a `RoomListService` (canon prohibition).
async fn refresh_room_list_from_joined_rooms(
    session: &MatrixClientSession,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    room_tx: &mpsc::Sender<RoomMessage>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) {
    let snapshot = koushi_sdk::room_list_snapshot_from_sdk_rooms_with_invites(
        session,
        session.client().joined_rooms(),
    )
    .await;
    relay_missing_space_child_links(&snapshot, room_tx).await;
    project_room_list_snapshot(&snapshot, known_room_ids, action_tx, event_tx).await;
}

/// SyncService-path observation loop (Async rule 1: relay the SDK's
/// observable streams). Subscribes to the live `RoomListService`'s
/// `all_rooms()` entries stream (`entries_with_dynamic_adapters` with the
/// non-left filter — the same shape the live service drives with its
/// `required_state`, including `m.room.create` for space classification) and
/// KEEPS CONSUMING it: the current entry vector is maintained by applying
/// each `VectorDiff` batch, and every visible joined/invited batch triggers a
/// re-normalization. The base client's committed room-update broadcast is a
/// second wake source for invite membership changes that do not alter the
/// bounded entries head; it never owns or drives another network sync.
/// The first batch (a Reset with the current entries) doubles as the initial
/// snapshot. A refresh request (operation-triggered) re-normalizes from the
/// current entries without touching the service. Exits on the oneshot stop
/// signal or when the stream ends.
async fn run_live_room_list_observation(
    session: Arc<MatrixClientSession>,
    service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    room_tx: mpsc::Sender<RoomMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    refresh_rx: mpsc::Receiver<()>,
    stop_rx: oneshot::Receiver<()>,
) {
    let room_updates_rx = session.client().subscribe_to_all_room_updates();
    #[cfg(test)]
    run_live_room_list_observation_with_sources(
        session,
        service,
        known_room_ids,
        room_tx,
        action_tx,
        event_tx,
        refresh_rx,
        stop_rx,
        ROOM_LIST_ENTRIES_LIMIT,
        room_updates_rx,
        None,
    )
    .await;
    #[cfg(not(test))]
    run_live_room_list_observation_with_sources(
        session,
        service,
        known_room_ids,
        room_tx,
        action_tx,
        event_tx,
        refresh_rx,
        stop_rx,
        ROOM_LIST_ENTRIES_LIMIT,
        room_updates_rx,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_live_room_list_observation_with_sources(
    session: Arc<MatrixClientSession>,
    service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    room_tx: mpsc::Sender<RoomMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut refresh_rx: mpsc::Receiver<()>,
    mut stop_rx: oneshot::Receiver<()>,
    entries_limit: usize,
    mut room_updates_rx: broadcast::Receiver<matrix_sdk_base::sync::RoomUpdates>,
    #[cfg(test)] test_event_tx: Option<mpsc::UnboundedSender<LiveObserverTestEvent>>,
) {
    use futures_util::StreamExt as _;

    let all_rooms = match service.all_rooms().await {
        Ok(all_rooms) => all_rooms,
        Err(_) => {
            record(
                DiagnosticEvent::new(DiagnosticLevel::Error, "core.room", "live_observer_exit")
                    .field(DiagnosticField::token("reason", "all_rooms_error")),
            );
            return;
        }
    };
    let (entries, entries_controller) = all_rooms.entries_with_dynamic_adapters(entries_limit);
    entries_controller.set_filter(Box::new(
        matrix_sdk_ui::room_list_service::filters::new_filter_non_left(),
    ));
    let mut entries = Box::pin(entries);
    let mut room_updates_closed = false;
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room", "live_observer_started").field(
            DiagnosticField::count("entries_limit", entries_limit as u64),
        ),
    );

    // Current filtered entry vector, maintained by applying each diff batch.
    let mut current: eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem> =
        eyeball_im::Vector::new();
    // `None` until the entries stream's initial Reset (or an explicit refresh)
    // has established the first projection. IDs remain private process state
    // and are never included in diagnostics.
    let mut projected_invite_ids: Option<BTreeSet<String>> = None;
    let mut rls_wake_count = 0_u64;
    let mut base_wake_count = 0_u64;

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                record_live_observer_exit(
                    DiagnosticLevel::Debug,
                    "stopped",
                    rls_wake_count,
                    base_wake_count,
                );
                break;
            },
            maybe_refresh = refresh_rx.recv() => {
                if maybe_refresh.is_none() {
                    record_live_observer_exit(
                        DiagnosticLevel::Error,
                        "refresh_channel_closed",
                        rls_wake_count,
                        base_wake_count,
                    );
                    break;
                }
                // Operation-triggered refresh: drain coalesced requests, then
                // re-normalize from the live service's CURRENT entries.
                while refresh_rx.try_recv().is_ok() {}
                projected_invite_ids = Some(normalize_and_project_entries(
                    &session,
                    &current,
                    &known_room_ids,
                    &room_tx,
                    &action_tx,
                    &event_tx,
                ).await.invite_ids);
            }
            maybe_diffs = entries.next() => match maybe_diffs {
                None => {
                    record_live_observer_exit(
                        DiagnosticLevel::Error,
                        "entries_stream_ended",
                        rls_wake_count,
                        base_wake_count,
                    );
                    break;
                },
                Some(diffs) => {
                    rls_wake_count = rls_wake_count.saturating_add(1);
                    for diff in diffs {
                        diff.apply(&mut current);
                    }
                    if rls_wake_count.is_power_of_two() {
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Debug,
                                "core.room",
                                "live_observer_wake_milestone",
                            )
                            .field(DiagnosticField::token("source", "rls_diff"))
                            .field(DiagnosticField::count("wake_count", rls_wake_count))
                            .field(DiagnosticField::count("entries_count", current.len() as u64)),
                        );
                    }
                    projected_invite_ids = Some(normalize_and_project_entries(
                        &session,
                        &current,
                        &known_room_ids,
                        &room_tx,
                        &action_tx,
                        &event_tx,
                    ).await.invite_ids);
                    #[cfg(test)]
                    emit_live_observer_test_event(
                        &test_event_tx,
                        LiveObserverTestEvent::RlsProjected {
                            wake_count: rls_wake_count,
                            entries_len: current.len(),
                        },
                    );
                }
            },
            room_update = room_updates_rx.recv(), if !room_updates_closed => {
                let mut update_count = 0_u64;
                let mut lagged = false;
                let mut invite_update_observed = false;
                match room_update {
                    Ok(updates) => {
                        update_count = 1;
                        invite_update_observed = !updates.invited.is_empty();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => lagged = true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        room_updates_closed = true;
                    }
                }
                loop {
                    match room_updates_rx.try_recv() {
                        Ok(updates) => {
                            update_count = update_count.saturating_add(1);
                            invite_update_observed |= !updates.invited.is_empty();
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                            lagged = true;
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                            room_updates_closed = true;
                            break;
                        }
                    }
                }

                if room_updates_closed {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Warn,
                            "core.room",
                            "live_observer_auxiliary_closed",
                        )
                        .field(DiagnosticField::token("source", "base_room_updates"))
                        .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
                        .field(DiagnosticField::count("base_wake_count", base_wake_count)),
                    );
                    #[cfg(test)]
                    emit_live_observer_test_event(
                        &test_event_tx,
                        LiveObserverTestEvent::BaseClosed,
                    );
                }

                base_wake_count = base_wake_count.saturating_add(1);
                let current_invite_ids = current_invite_membership(&session);
                let invite_membership_changed = projected_invite_ids
                    .as_ref()
                    .is_some_and(|projected| projected != &current_invite_ids);
                let projection_required = invite_projection_required(
                    projected_invite_ids.as_ref(),
                    &current_invite_ids,
                    invite_update_observed,
                    lagged,
                );
                if base_wake_count.is_power_of_two() {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Debug,
                            "core.room",
                            "live_observer_wake_milestone",
                        )
                        .field(DiagnosticField::token("source", "base_room_updates"))
                        .field(DiagnosticField::count("wake_count", base_wake_count))
                        .field(DiagnosticField::count("drained_update_count", update_count))
                        .field(DiagnosticField::boolean("lagged", lagged))
                        .field(DiagnosticField::boolean(
                            "invite_update_observed",
                            invite_update_observed,
                        ))
                        .field(DiagnosticField::boolean(
                            "initial_projection_complete",
                            projected_invite_ids.is_some(),
                        ))
                        .field(DiagnosticField::boolean(
                            "invite_membership_changed",
                            invite_membership_changed,
                        ))
                        .field(DiagnosticField::boolean(
                            "projection_required",
                            projection_required,
                        )),
                    );
                }
                #[cfg(test)]
                emit_live_observer_test_event(
                    &test_event_tx,
                    LiveObserverTestEvent::BaseBatch {
                        wake_count: base_wake_count,
                        update_count,
                        lagged,
                        projection_required,
                    },
                );
                if lagged {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Warn,
                            "core.room",
                            "live_observer_base_lagged",
                        )
                        .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
                        .field(DiagnosticField::count("base_wake_count", base_wake_count))
                        .field(DiagnosticField::count("drained_update_count", update_count)),
                    );
                }

                if projection_required {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Debug,
                            "core.room",
                            "live_observer_invite_projection",
                        )
                        .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
                        .field(DiagnosticField::count("base_wake_count", base_wake_count))
                        .field(DiagnosticField::count("drained_update_count", update_count))
                        .field(DiagnosticField::boolean("lagged", lagged))
                        .field(DiagnosticField::boolean(
                            "invite_update_observed",
                            invite_update_observed,
                        ))
                        .field(DiagnosticField::boolean(
                            "invite_membership_changed",
                            invite_membership_changed,
                        )),
                    );
                    let projection = normalize_and_project_entries(
                        &session,
                        &current,
                        &known_room_ids,
                        &room_tx,
                        &action_tx,
                        &event_tx,
                    ).await;
                    let action_delivered = projection.action_delivered;
                    projected_invite_ids = Some(projection.invite_ids);
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Debug,
                            "core.room",
                            "live_observer_invite_projection_completed",
                        )
                        .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
                        .field(DiagnosticField::count("base_wake_count", base_wake_count))
                        .field(DiagnosticField::boolean(
                            "action_delivered",
                            action_delivered,
                        )),
                    );
                    #[cfg(test)]
                    emit_live_observer_test_event(
                        &test_event_tx,
                        LiveObserverTestEvent::BaseProjected {
                            wake_count: base_wake_count,
                            rls_wake_count,
                            entries_len: current.len(),
                            action_delivered,
                        },
                    );
                }
            }
        }
    }
}

fn record_live_observer_exit(
    level: DiagnosticLevel,
    reason: &'static str,
    rls_wake_count: u64,
    base_wake_count: u64,
) {
    record(
        DiagnosticEvent::new(level, "core.room", "live_observer_exit")
            .field(DiagnosticField::token("reason", reason))
            .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
            .field(DiagnosticField::count("base_wake_count", base_wake_count)),
    );
}

fn current_invite_membership(session: &MatrixClientSession) -> BTreeSet<String> {
    session
        .client()
        .invited_rooms()
        .into_iter()
        .map(|room| room.room_id().to_string())
        .collect()
}

fn invite_projection_required(
    projected_invite_ids: Option<&BTreeSet<String>>,
    current_invite_ids: &BTreeSet<String>,
    invite_update_observed: bool,
    lagged: bool,
) -> bool {
    projected_invite_ids.is_some_and(|projected| {
        projected != current_invite_ids || invite_update_observed || lagged
    })
}

struct RoomListProjectionResult {
    invite_ids: BTreeSet<String>,
    action_delivered: bool,
}

/// Normalize the live service's current entries and project the result.
async fn normalize_and_project_entries(
    session: &MatrixClientSession,
    current: &eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    room_tx: &mpsc::Sender<RoomMessage>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) -> RoomListProjectionResult {
    // Collect before the await: mapping lazily across the await trips a
    // higher-ranked lifetime check on the iterator closure.
    let mut rooms = Vec::with_capacity(current.len());
    for item in current.iter() {
        rooms.push(item.clone().into_inner());
    }
    let snapshot = koushi_sdk::room_list_snapshot_from_sdk_rooms_with_invites(session, rooms).await;
    let projected_invite_ids = snapshot
        .invites
        .iter()
        .map(|invite| invite.room_id.clone())
        .collect();
    relay_missing_space_child_links(&snapshot, room_tx).await;
    let action_delivered =
        project_room_list_snapshot(&snapshot, known_room_ids, action_tx, event_tx).await;
    RoomListProjectionResult {
        invite_ids: projected_invite_ids,
        action_delivered,
    }
}

async fn relay_missing_space_child_links(
    snapshot: &MatrixRoomListSnapshot,
    room_tx: &mpsc::Sender<RoomMessage>,
) {
    let links = missing_space_child_links(snapshot);
    if !links.is_empty() {
        let _ = room_tx
            .send(RoomMessage::MissingSpaceChildLinks { links })
            .await;
    }
}

fn missing_space_child_links(snapshot: &MatrixRoomListSnapshot) -> Vec<MissingSpaceChildLink> {
    let mut links = Vec::new();
    for room in &snapshot.rooms {
        for space in &snapshot.spaces {
            if room_has_parent_without_space_child(room, space)
                && let Ok(via_server) = koushi_sdk::room_id_server_name(&room.room_id)
            {
                links.push(MissingSpaceChildLink {
                    space_id: space.space_id.clone(),
                    child_room_id: room.room_id.clone(),
                    via_server,
                });
            }
        }
    }
    links.sort_by(|left, right| {
        left.space_id
            .cmp(&right.space_id)
            .then_with(|| left.child_room_id.cmp(&right.child_room_id))
    });
    links.dedup_by(|left, right| {
        left.space_id == right.space_id && left.child_room_id == right.child_room_id
    });
    links
}

fn room_has_parent_without_space_child(
    room: &MatrixRoomListRoom,
    space: &MatrixRoomListSpace,
) -> bool {
    room.parent_space_ids
        .iter()
        .any(|space_id| space_id == &space.space_id)
        && !space
            .child_room_ids
            .iter()
            .any(|child_room_id| child_room_id == &room.room_id)
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
    room_tx: mpsc::Sender<RoomMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut refresh_rx: mpsc::Receiver<()>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    use tokio::sync::broadcast::error::RecvError;

    let mut updates_rx = session.client().subscribe_to_all_room_updates();
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            maybe_refresh = refresh_rx.recv() => {
                if maybe_refresh.is_none() {
                    break;
                }
                // Operation-triggered refresh: drain coalesced requests, then
                // normalize from the SDK's current joined-room snapshot.
                while refresh_rx.try_recv().is_ok() {}
                refresh_room_list_from_joined_rooms(
                    &session,
                    &known_room_ids,
                    &room_tx,
                    &action_tx,
                    &event_tx,
                ).await;
            }
            result = updates_rx.recv() => match result {
                Ok(_batch) => {
                    // Coalesce: drain any additionally pending update batches;
                    // one refresh covers them all.
                    while updates_rx.try_recv().is_ok() {}
                    refresh_room_list_from_joined_rooms(
                        &session,
                        &known_room_ids,
                        &room_tx,
                        &action_tx,
                        &event_tx,
                    ).await;
                }
                Err(RecvError::Lagged(_)) => {
                    // The snapshot is self-healing: refresh once.
                    refresh_room_list_from_joined_rooms(
                        &session,
                        &known_room_ids,
                        &room_tx,
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
/// child room id lists. Homeservers may sync one side of the Matrix space
/// relationship before the other, so the projection uses both the space's
/// `m.space.child` state and rooms' `m.space.parent` state.
fn normalize_spaces(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<SpaceSummary> {
    snapshot
        .spaces
        .iter()
        .map(|space| {
            let child_room_ids = normalize_space_child_room_ids(snapshot, space);
            SpaceSummary {
                space_id: space.space_id.clone(),
                display_name: space.display_name.clone(),
                avatar: avatar_from_mxc_uri(space.avatar_mxc_uri.as_deref()),
                child_room_ids,
            }
        })
        .collect()
}

fn normalize_space_child_room_ids(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    space: &koushi_sdk::MatrixRoomListSpace,
) -> Vec<String> {
    let mut child_room_ids = BTreeSet::new();
    child_room_ids.extend(space.child_room_ids.iter().cloned());
    child_room_ids.extend(
        snapshot
            .rooms
            .iter()
            .filter(|room| room.parent_space_ids.iter().any(|id| id == &space.space_id))
            .map(|room| room.room_id.clone()),
    );
    child_room_ids.into_iter().collect()
}

/// Convert `MatrixRoomListSnapshot` rooms into `RoomSummary` values.
fn normalize_rooms(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<RoomSummary> {
    let mut rooms: Vec<RoomSummary> = snapshot
        .rooms
        .iter()
        .map(|room| {
            let display_label = room
                .display_name
                .trim()
                .is_empty()
                .then(|| room.room_id.clone())
                .unwrap_or_else(|| room.display_name.trim().to_owned());
            RoomSummary {
                room_id: room.room_id.clone(),
                display_name: room.display_name.clone(),
                display_label: display_label.clone(),
                original_display_label: display_label,
                avatar: avatar_from_mxc_uri(room.avatar_mxc_uri.as_deref()),
                is_dm: room.is_dm,
                dm_user_ids: room.dm_user_ids.clone(),
                tags: normalize_room_tags(&room.tags),
                unread_count: room.unread_count,
                notification_count: room.notification_count,
                highlight_count: room.highlight_count,
                marked_unread: room.marked_unread,
                recency_stamp: room.recency_stamp,
                conversation_activity: room.conversation_activity.map(|activity| {
                    koushi_state::ConversationActivity {
                        timestamp_ms: activity.timestamp_ms,
                        source: match activity.source {
                            koushi_sdk::MatrixConversationActivitySource::Message => {
                                koushi_state::ConversationActivitySource::Message
                            }
                            koushi_sdk::MatrixConversationActivitySource::EncryptedMessage => {
                                koushi_state::ConversationActivitySource::EncryptedMessage
                            }
                            koushi_sdk::MatrixConversationActivitySource::ThreadReply => {
                                koushi_state::ConversationActivitySource::ThreadReply
                            }
                        },
                    }
                }),
                latest_event: room.latest_event.as_ref().map(|event| {
                    koushi_state::RoomLatestEventSummary {
                        event_id: event.event_id.clone(),
                        sender_id: event.sender_id.clone(),
                        sender_label: event.sender_label.clone(),
                        sender_avatar: avatar_from_mxc_uri(event.sender_avatar_mxc_uri.as_deref()),
                        preview: event.preview.clone(),
                        timestamp_ms: event.timestamp_ms,
                    }
                }),
                parent_space_ids: normalize_room_parent_space_ids(snapshot, room),
                dm_space_ids: Vec::new(),
                is_encrypted: room.is_encrypted,
                joined_members: room.joined_members,
            }
        })
        .collect();
    let space_members: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        snapshot
            .spaces
            .iter()
            .map(|s| {
                (
                    s.space_id.clone(),
                    s.member_user_ids.iter().cloned().collect(),
                )
            })
            .collect();
    assign_dm_space_ids(&mut rooms, &space_members);
    rooms
}

fn normalize_room_parent_space_ids(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    room: &koushi_sdk::MatrixRoomListRoom,
) -> Vec<String> {
    let mut parent_space_ids: BTreeSet<String> = room.parent_space_ids.iter().cloned().collect();
    parent_space_ids.extend(
        snapshot
            .spaces
            .iter()
            .filter(|space| space.child_room_ids.iter().any(|id| id == &room.room_id))
            .map(|space| space.space_id.clone()),
    );
    parent_space_ids.into_iter().collect()
}

/// Populate `dm_space_ids` on each `RoomSummary` in `rooms`.
///
/// For each DM room, `dm_space_ids` is set to the sorted list of space IDs
/// (keys of `space_members`) whose member set contains at least one of
/// `room.dm_user_ids`. Non-DM rooms always get an empty `dm_space_ids`.
///
/// The result is deterministically ordered because `space_members` is a
/// `BTreeMap` and iteration yields keys in ascending order.
pub fn assign_dm_space_ids(
    rooms: &mut [koushi_state::RoomSummary],
    space_members: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) {
    for room in rooms.iter_mut() {
        if !room.is_dm {
            room.dm_space_ids = Vec::new();
            continue;
        }
        room.dm_space_ids = space_members
            .iter()
            .filter(|(_space_id, members)| room.dm_user_ids.iter().any(|uid| members.contains(uid)))
            .map(|(space_id, _)| space_id.clone())
            .collect();
    }
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

fn normalize_user_profiles(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<UserProfile> {
    snapshot
        .user_profiles
        .iter()
        .map(|profile| {
            let display_label = profile
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|display_name| !display_name.is_empty())
                .unwrap_or(profile.user_id.as_str())
                .to_owned();
            UserProfile {
                user_id: profile.user_id.clone(),
                display_name: profile.display_name.clone(),
                display_label: display_label.clone(),
                original_display_label: display_label,
                mention_search_terms: user_profile_mention_search_terms(
                    &profile.user_id,
                    profile.display_name.as_deref(),
                ),
                avatar: avatar_from_mxc_uri(profile.avatar_mxc_uri.as_deref()),
            }
        })
        .collect()
}

fn user_profile_mention_search_terms(user_id: &str, display_name: Option<&str>) -> Vec<String> {
    let mut terms = Vec::new();
    if let Some(display_name) = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        terms.push(display_name.to_owned());
    }
    if !terms.iter().any(|term| term == user_id) {
        terms.push(user_id.to_owned());
    }
    terms
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
fn normalize_invites(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<InvitePreview> {
    snapshot
        .invites
        .iter()
        .map(|invite| InvitePreview {
            room_id: invite.room_id.clone(),
            display_name: invite.display_name.clone(),
            avatar: avatar_from_mxc_uri(invite.avatar_mxc_uri.as_deref()),
            topic: invite.topic.clone(),
            inviter_display_name: invite.inviter_display_name.clone(),
            inviter_user_id: invite.inviter_user_id.clone(),
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

fn matrix_create_room_options(options: CreateRoomOptions) -> MatrixCreateRoomOptions {
    MatrixCreateRoomOptions {
        name: options.name,
        topic: options.topic,
        alias_localpart: options.alias_localpart,
        encrypted: options.encrypted,
        visibility: match options.visibility {
            CreateRoomVisibility::Private => MatrixCreateRoomVisibility::Private,
            CreateRoomVisibility::Public => MatrixCreateRoomVisibility::Public,
        },
        parent_space: options
            .parent_space
            .map(|parent| MatrixCreateRoomParentSpace {
                space_id: parent.space_id,
                via_server: parent.via_server,
            }),
    }
}

fn room_settings_snapshot_from_sdk(settings: MatrixRoomSettingsSnapshot) -> RoomSettingsSnapshot {
    let share_link = koushi_state::room_settings_share_link(
        &settings.room_id,
        settings.canonical_alias.as_deref(),
        &settings.alternate_aliases,
    );
    RoomSettingsSnapshot {
        room_id: settings.room_id,
        name: settings.name,
        topic: settings.topic,
        avatar_url: settings.avatar_url,
        canonical_alias: settings.canonical_alias,
        alternate_aliases: settings.alternate_aliases,
        share_link,
        join_rule: room_join_rule_from_sdk(settings.join_rule),
        history_visibility: room_history_visibility_from_sdk(settings.history_visibility),
        permissions: room_permission_facts_from_sdk(settings.permissions),
        members: settings
            .members
            .into_iter()
            .map(room_member_summary_from_sdk)
            .collect(),
    }
}

fn room_member_summary_from_sdk(member: MatrixRoomMemberSummary) -> RoomMemberSummary {
    let display_label = member
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty())
        .unwrap_or(member.user_id.as_str())
        .to_owned();
    RoomMemberSummary {
        user_id: member.user_id,
        display_name: member.display_name,
        display_label: display_label.clone(),
        original_display_label: display_label,
        avatar_url: member.avatar_url,
        power_level: member.power_level,
        role: room_member_role_from_sdk(member.role),
        user_trust: member.user_trust.map(user_trust_state_from_sdk),
    }
}

fn user_trust_state_from_sdk(state: MatrixUserTrustState) -> UserTrustState {
    match state {
        MatrixUserTrustState::Unverified => UserTrustState::Unverified,
        MatrixUserTrustState::Verified => UserTrustState::Verified,
        MatrixUserTrustState::IdentityReset => UserTrustState::IdentityReset,
    }
}

fn room_member_role_from_sdk(role: MatrixRoomMemberRole) -> RoomMemberRole {
    match role {
        MatrixRoomMemberRole::Creator => RoomMemberRole::Creator,
        MatrixRoomMemberRole::Administrator => RoomMemberRole::Administrator,
        MatrixRoomMemberRole::Moderator => RoomMemberRole::Moderator,
        MatrixRoomMemberRole::User => RoomMemberRole::User,
    }
}

fn room_join_rule_from_sdk(join_rule: MatrixRoomJoinRule) -> RoomJoinRule {
    match join_rule {
        MatrixRoomJoinRule::Public => RoomJoinRule::Public,
        MatrixRoomJoinRule::Invite => RoomJoinRule::Invite,
        MatrixRoomJoinRule::Knock => RoomJoinRule::Knock,
        MatrixRoomJoinRule::Restricted => RoomJoinRule::Restricted,
        MatrixRoomJoinRule::Private => RoomJoinRule::Private,
    }
}

fn room_join_rule_to_sdk(join_rule: RoomJoinRule) -> MatrixRoomJoinRule {
    match join_rule {
        RoomJoinRule::Public => MatrixRoomJoinRule::Public,
        RoomJoinRule::Invite => MatrixRoomJoinRule::Invite,
        RoomJoinRule::Knock => MatrixRoomJoinRule::Knock,
        RoomJoinRule::Restricted => MatrixRoomJoinRule::Restricted,
        RoomJoinRule::Private => MatrixRoomJoinRule::Private,
    }
}

fn room_history_visibility_from_sdk(
    history_visibility: MatrixRoomHistoryVisibility,
) -> RoomHistoryVisibility {
    match history_visibility {
        MatrixRoomHistoryVisibility::WorldReadable => RoomHistoryVisibility::WorldReadable,
        MatrixRoomHistoryVisibility::Shared => RoomHistoryVisibility::Shared,
        MatrixRoomHistoryVisibility::Invited => RoomHistoryVisibility::Invited,
        MatrixRoomHistoryVisibility::Joined => RoomHistoryVisibility::Joined,
    }
}

fn room_history_visibility_to_sdk(
    history_visibility: RoomHistoryVisibility,
) -> MatrixRoomHistoryVisibility {
    match history_visibility {
        RoomHistoryVisibility::WorldReadable => MatrixRoomHistoryVisibility::WorldReadable,
        RoomHistoryVisibility::Shared => MatrixRoomHistoryVisibility::Shared,
        RoomHistoryVisibility::Invited => MatrixRoomHistoryVisibility::Invited,
        RoomHistoryVisibility::Joined => MatrixRoomHistoryVisibility::Joined,
    }
}

fn room_permission_facts_from_sdk(permissions: MatrixRoomPermissionFacts) -> RoomPermissionFacts {
    RoomPermissionFacts {
        can_edit_settings: permissions.can_edit_settings,
        can_edit_roles: permissions.can_edit_roles,
        can_kick: permissions.can_kick,
        can_ban: permissions.can_ban,
        can_unban: permissions.can_unban,
    }
}

fn room_setting_change_to_sdk(change: RoomSettingChange) -> MatrixRoomSettingChange {
    match change {
        RoomSettingChange::Name(name) => MatrixRoomSettingChange::Name(name),
        RoomSettingChange::Topic(topic) => MatrixRoomSettingChange::Topic(topic),
        RoomSettingChange::AvatarUrl(avatar_url) => MatrixRoomSettingChange::AvatarUrl(avatar_url),
        RoomSettingChange::JoinRule(join_rule) => {
            MatrixRoomSettingChange::JoinRule(room_join_rule_to_sdk(join_rule))
        }
        RoomSettingChange::HistoryVisibility(history_visibility) => {
            MatrixRoomSettingChange::HistoryVisibility(room_history_visibility_to_sdk(
                history_visibility,
            ))
        }
    }
}

fn room_moderation_action_to_sdk(action: RoomModerationAction) -> MatrixRoomModerationAction {
    match action {
        RoomModerationAction::Kick => MatrixRoomModerationAction::Kick,
        RoomModerationAction::Ban => MatrixRoomModerationAction::Ban,
        RoomModerationAction::Unban => MatrixRoomModerationAction::Unban,
    }
}

fn room_moderation_allowed(
    permissions: &RoomPermissionFacts,
    action: RoomModerationAction,
) -> bool {
    match action {
        RoomModerationAction::Kick => permissions.can_kick,
        RoomModerationAction::Ban => permissions.can_ban,
        RoomModerationAction::Unban => permissions.can_unban,
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

enum InviteTargetOutcome {
    Invited,
    AlreadyInSpace,
    Failed,
}

async fn invite_target_to_space_if_needed(
    session: &MatrixClientSession,
    space_id: &str,
    user_id: &str,
) -> InviteTargetOutcome {
    match koushi_sdk::room_has_active_member_no_sync(session, space_id, user_id).await {
        Ok(true) => return InviteTargetOutcome::AlreadyInSpace,
        Ok(false) => {}
        Err(_error) => return InviteTargetOutcome::Failed,
    }

    match koushi_sdk::invite_user_to_room(session, space_id, user_id).await {
        Ok(()) => InviteTargetOutcome::Invited,
        Err(_error) => InviteTargetOutcome::Failed,
    }
}

// ---------------------------------------------------------------------------
// Error classification (never raw SDK text in public events)
// ---------------------------------------------------------------------------

/// Map a `MatrixRoomOperationError` to a coarse `RoomFailureKind`.
/// The spec defines: Forbidden / NotFound / Network / Sdk.
/// Raw SDK error text must never appear in public events.
pub(crate) fn classify_room_error(error: &MatrixRoomOperationError) -> RoomFailureKind {
    use koushi_sdk::MatrixRoomOperationFailureKind;
    match error {
        MatrixRoomOperationError::InvalidRoomSetting => RoomFailureKind::Sdk,
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

fn classify_report_error(
    error: &koushi_sdk::MatrixReportError,
) -> crate::failure::ReportFailureKind {
    use crate::failure::ReportFailureKind;
    use koushi_sdk::MatrixReportFailureKind;
    match error.failure_kind() {
        MatrixReportFailureKind::Forbidden => ReportFailureKind::Forbidden,
        MatrixReportFailureKind::Network => ReportFailureKind::Network,
        MatrixReportFailureKind::InvalidUserId => ReportFailureKind::InvalidUserId,
        MatrixReportFailureKind::InvalidRoomId => ReportFailureKind::InvalidRoomId,
        MatrixReportFailureKind::InvalidEventId => ReportFailureKind::InvalidEventId,
        MatrixReportFailureKind::Sdk => ReportFailureKind::Sdk,
    }
}

fn trace_room_operation(kind: &'static str, stage: &'static str, request_id: RequestId) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room", stage)
            .field(DiagnosticField::token("operation", kind))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            )),
    );
}

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use koushi_sdk::{
        MatrixConversationActivity, MatrixConversationActivitySource, MatrixInvitePreview,
        MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace, MatrixRoomMemberRole,
        MatrixRoomPermissionFacts, MatrixRoomSettingsSnapshot, MatrixRoomTagInfo, MatrixRoomTags,
    };
    use koushi_state::{RoomMemberRole, RoomTagInfo, RoomTagKind, SessionInfo};
    use tokio::sync::{broadcast, mpsc, oneshot};

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

    async fn wait_for_live_observer_test_event(
        rx: &mut mpsc::UnboundedReceiver<LiveObserverTestEvent>,
        label: &'static str,
        predicate: impl Fn(&LiveObserverTestEvent) -> bool,
    ) -> LiveObserverTestEvent {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let event = rx.recv().await.expect("live observer test channel");
                if predicate(&event) {
                    break event;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {label}"))
    }

    struct LiveObserverTestHarness {
        action_rx: mpsc::Receiver<Vec<AppAction>>,
        test_event_rx: mpsc::UnboundedReceiver<LiveObserverTestEvent>,
        _refresh_tx: mpsc::Sender<()>,
        stop_tx: oneshot::Sender<()>,
        task: tokio::task::JoinHandle<()>,
    }

    impl LiveObserverTestHarness {
        async fn next_actions(&mut self, label: &'static str) -> Vec<AppAction> {
            tokio::time::timeout(Duration::from_secs(1), self.action_rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {label}"))
                .expect("action channel should stay open")
        }

        async fn expect_event(&mut self, label: &'static str, expected: LiveObserverTestEvent) {
            let actual =
                wait_for_live_observer_test_event(&mut self.test_event_rx, label, |event| {
                    event == &expected
                })
                .await;
            assert_eq!(actual, expected);
        }

        async fn stop(self) {
            let _ = self.stop_tx.send(());
            self.task.await.expect("observer task");
        }
    }

    async fn spawn_live_observer_test_harness(
        client: matrix_sdk::Client,
        homeserver: String,
        entries_limit: usize,
        room_updates_rx: broadcast::Receiver<matrix_sdk_base::sync::RoomUpdates>,
    ) -> LiveObserverTestHarness {
        let service = Arc::new(
            matrix_sdk_ui::room_list_service::RoomListService::new(client.clone())
                .await
                .expect("room list service"),
        );
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver,
                user_id: "@observer:example.invalid".to_owned(),
                device_id: "OBSERVER".to_owned(),
            },
        ));
        let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
        let (room_tx, _room_rx) = mpsc::channel(4);
        let (action_tx, action_rx) = mpsc::channel(8);
        let (event_tx, _event_rx) = broadcast::channel(8);
        let (refresh_tx, refresh_rx) = mpsc::channel(1);
        let (stop_tx, stop_rx) = oneshot::channel();
        let (test_event_tx, test_event_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_live_room_list_observation_with_sources(
            session,
            service,
            known_room_ids,
            room_tx,
            action_tx,
            event_tx,
            refresh_rx,
            stop_rx,
            entries_limit,
            room_updates_rx,
            Some(test_event_tx),
        ));
        LiveObserverTestHarness {
            action_rx,
            test_event_rx,
            _refresh_tx: refresh_tx,
            stop_tx,
            task,
        }
    }

    #[test]
    fn room_operation_records_without_environment_switch() {
        trace_room_operation("create_room", "test_always_on", make_request_id(999));
        assert!(koushi_diagnostics::snapshot().records.iter().any(|record| {
            record.event.source == "core.room" && record.event.stage == "test_always_on"
        }));
    }

    #[test]
    fn invite_projection_policy_self_heals_after_lag_and_skips_ordinary_updates() {
        let projected = BTreeSet::from(["!invite:example.invalid".to_owned()]);
        let changed = BTreeSet::from(["!other-invite:example.invalid".to_owned()]);

        assert!(!invite_projection_required(
            Some(&projected),
            &projected,
            false,
            false,
        ));
        assert!(invite_projection_required(
            Some(&projected),
            &projected,
            true,
            false,
        ));
        assert!(invite_projection_required(
            Some(&projected),
            &projected,
            false,
            true,
        ));
        assert!(invite_projection_required(
            Some(&projected),
            &changed,
            false,
            false,
        ));
        assert!(!invite_projection_required(None, &changed, true, true,));
    }

    // --- Error classification ---

    #[test]
    fn forbidden_sdk_error_classifies_as_forbidden() {
        let error =
            MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::Forbidden);
        assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
    }

    #[test]
    fn auth_required_sdk_error_classifies_as_forbidden() {
        let error = MatrixRoomOperationError::Sdk(
            koushi_sdk::MatrixRoomOperationFailureKind::AuthenticationRequired,
        );
        assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
    }

    #[test]
    fn http_sdk_error_classifies_as_network() {
        let error = MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::Http);
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
        let error = MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::Sdk);
        assert_eq!(classify_room_error(&error), RoomFailureKind::Sdk);
    }

    #[test]
    fn mark_room_as_read_success_updates_fully_read_marker_before_clearing_counts() {
        let source = include_str!("room.rs");
        let handler = source
            .split("async fn handle_mark_room_as_read")
            .nth(1)
            .expect("handle_mark_room_as_read should exist")
            .split("async fn handle_mark_room_as_unread")
            .next()
            .expect("handle_mark_room_as_unread should follow handle_mark_room_as_read");
        let success_arm = handler
            .split("Ok(()) => {")
            .nth(1)
            .expect("mark read success arm should exist")
            .split("Err(error) => {")
            .next()
            .expect("mark read error arm should follow success arm");

        assert!(
            success_arm.contains("AppAction::FullyReadMarkerUpdated"),
            "mark-room-as-read success must update local fully-read state so stale room-list snapshots cannot resurrect unread counts"
        );
        assert!(
            success_arm.contains("AppAction::RoomMarkedAsReadSucceeded"),
            "mark-room-as-read success must still clear room summary unread counts"
        );
        assert!(
            success_arm.find("FullyReadMarkerUpdated")
                < success_arm.find("RoomMarkedAsReadSucceeded"),
            "fully-read marker should be reduced before unread counts are cleared"
        );
    }

    #[test]
    fn room_settings_snapshot_mapping_preserves_role_power_and_role_permission_facts() {
        let settings = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Private room".to_owned()),
            topic: Some("Private topic".to_owned()),
            avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
            canonical_alias: Some("#private:example.invalid".to_owned()),
            alternate_aliases: vec!["#alternate:example.invalid".to_owned()],
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: false,
                can_unban: false,
            },
            members: vec![MatrixRoomMemberSummary {
                user_id: "@member:example.invalid".to_owned(),
                display_name: Some("Private member".to_owned()),
                avatar_url: Some("mxc://example.invalid/member-avatar".to_owned()),
                power_level: Some(50),
                role: MatrixRoomMemberRole::Moderator,
                user_trust: None,
            }],
        };

        let mapped = room_settings_snapshot_from_sdk(settings);

        assert!(mapped.permissions.can_edit_roles);
        assert_eq!(
            mapped.share_link.as_deref(),
            Some("https://matrix.to/#/%23private%3Aexample.invalid")
        );
        let member = mapped.members.first().expect("member summary");
        assert_eq!(member.power_level, Some(50));
        assert_eq!(member.role, RoomMemberRole::Moderator);
        let debug = format!("{mapped:?}");
        assert!(!debug.contains("Private room"), "{debug}");
        assert!(!debug.contains("Private topic"), "{debug}");
        assert!(!debug.contains("@member:example.invalid"), "{debug}");
        assert!(!debug.contains("mxc://example.invalid"), "{debug}");
    }

    // --- Room list normalization: spaces ---

    #[test]
    fn normalize_rooms_preserves_typed_conversation_activity_and_opaque_recency() {
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!dm:example.test".to_owned(),
                display_name: "Synthetic DM".to_owned(),
                avatar_mxc_uri: None,
                is_dm: true,
                dm_user_ids: vec!["@member:example.test".to_owned()],
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: Some(9),
                conversation_activity: Some(MatrixConversationActivity {
                    timestamp_ms: 42,
                    source: MatrixConversationActivitySource::EncryptedMessage,
                }),
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: true,
                joined_members: 2,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let rooms = normalize_rooms(&snapshot);
        let room = rooms.first().expect("normalized room");

        assert_eq!(room.recency_stamp, Some(9));
        assert_eq!(
            room.conversation_activity,
            Some(koushi_state::ConversationActivity {
                timestamp_ms: 42,
                source: koushi_state::ConversationActivitySource::EncryptedMessage,
            })
        );
    }

    #[test]
    fn normalize_spaces_with_child_rooms() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space1:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
            }],
            rooms: vec![
                MatrixRoomListRoom {
                    room_id: "!room1:example.test".to_owned(),
                    display_name: "Room 1".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: false,
                    dm_user_ids: Vec::new(),
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: vec!["!space1:example.test".to_owned()],
                    is_encrypted: false,
                    joined_members: 0,
                },
                MatrixRoomListRoom {
                    room_id: "!room2:example.test".to_owned(),
                    display_name: "Room 2".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: false,
                    dm_user_ids: Vec::new(),
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: vec![],
                    is_encrypted: false,
                    joined_members: 0,
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
    fn missing_space_child_links_detects_parent_only_relationship() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
            }],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "Room".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec!["!space:example.test".to_owned()],
                is_encrypted: true,
                joined_members: 1,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        assert_eq!(
            missing_space_child_links(&snapshot),
            vec![MissingSpaceChildLink {
                space_id: "!space:example.test".to_owned(),
                child_room_id: "!room:example.test".to_owned(),
                via_server: "example.test".to_owned(),
            }]
        );
    }

    #[test]
    fn missing_space_child_links_skips_reciprocal_relationship() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: vec!["!room:example.test".to_owned()],
                member_user_ids: Vec::new(),
            }],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "Room".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec!["!space:example.test".to_owned()],
                is_encrypted: true,
                joined_members: 1,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        assert!(missing_space_child_links(&snapshot).is_empty());
    }

    #[test]
    fn normalize_spaces_uses_direct_space_child_state() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space1:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: vec!["!room1:example.test".to_owned()],
                member_user_ids: Vec::new(),
            }],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room1:example.test".to_owned(),
                display_name: "Room 1".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let spaces = normalize_spaces(&snapshot);

        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].child_room_ids, vec!["!room1:example.test"]);
    }

    #[test]
    fn normalize_spaces_no_children() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Empty Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
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
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
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
                dm_user_ids: vec!["@alice:example.test".to_owned()],
                tags: MatrixRoomTags::default(),
                unread_count: 3,
                notification_count: 3,
                highlight_count: 1,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
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
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec!["!space:example.test".to_owned()],
                is_encrypted: false,
                joined_members: 0,
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
    fn normalize_rooms_uses_direct_space_child_state_as_parent() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: vec!["!room:example.test".to_owned()],
                member_user_ids: Vec::new(),
            }],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let rooms = normalize_rooms(&snapshot);

        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].parent_space_ids, vec!["!space:example.test"]);
    }

    #[test]
    fn normalize_rooms_assigns_dm_space_ids_by_counterpart_membership() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "space-a".to_owned(),
                display_name: "Space A".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: vec!["@alice".to_owned()],
            }],
            rooms: vec![
                MatrixRoomListRoom {
                    room_id: "dm-alice".to_owned(),
                    display_name: "Alice".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: true,
                    dm_user_ids: vec!["@alice".to_owned()],
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: Vec::new(),
                    is_encrypted: false,
                    joined_members: 0,
                },
                MatrixRoomListRoom {
                    room_id: "dm-bob".to_owned(),
                    display_name: "Bob".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: true,
                    dm_user_ids: vec!["@bob".to_owned()],
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: Vec::new(),
                    is_encrypted: false,
                    joined_members: 0,
                },
            ],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);
        let alice_room = rooms.iter().find(|r| r.room_id == "dm-alice").unwrap();
        let bob_room = rooms.iter().find(|r| r.room_id == "dm-bob").unwrap();
        assert_eq!(alice_room.dm_space_ids, vec!["space-a"]);
        assert_eq!(bob_room.dm_space_ids, Vec::<String>::new());
    }

    #[test]
    fn normalize_rooms_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/room-avatar".to_owned()),
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
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
                inviter_user_id: Some("@inviter:example.test".to_owned()),
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
                inviter_user_id: None,
                is_dm: false,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let invites = normalize_invites(&snapshot);

        let avatar = invites[0].avatar.as_ref().expect("invite avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/invite-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn normalize_user_profiles_preserves_member_profile_fields() {
        let snapshot = MatrixRoomListSnapshot {
            user_profiles: vec![koushi_sdk::MatrixUserProfile {
                user_id: "@alice:example.test".to_owned(),
                display_name: Some("Alice".to_owned()),
                avatar_mxc_uri: Some("mxc://example.test/alice".to_owned()),
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let profiles = normalize_user_profiles(&snapshot);

        assert_eq!(
            profiles,
            vec![UserProfile {
                user_id: "@alice:example.test".to_owned(),
                display_name: Some("Alice".to_owned()),
                display_label: "Alice".to_owned(),
                original_display_label: "Alice".to_owned(),
                mention_search_terms: vec!["Alice".to_owned(), "@alice:example.test".to_owned(),],
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.test/alice".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            }]
        );
    }

    #[tokio::test]
    async fn project_room_list_snapshot_updates_user_profiles() {
        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
        let snapshot = MatrixRoomListSnapshot {
            user_profiles: vec![koushi_sdk::MatrixUserProfile {
                user_id: "@alice:example.test".to_owned(),
                display_name: Some("Alice".to_owned()),
                avatar_mxc_uri: None,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        project_room_list_snapshot(&snapshot, &known_room_ids, &action_tx, &event_tx).await;

        let actions = action_rx.recv().await.expect("actions");
        assert!(
            matches!(
                actions.as_slice(),
                [
                    AppAction::RoomListUpdated { .. },
                    AppAction::InviteListUpdated { .. },
                    AppAction::UserProfilesUpdated { profiles },
                ] if profiles == &vec![UserProfile {
                    user_id: "@alice:example.test".to_owned(),
                    display_name: Some("Alice".to_owned()),
                    display_label: "Alice".to_owned(),
                    original_display_label: "Alice".to_owned(),
                    mention_search_terms: vec![
                        "Alice".to_owned(),
                        "@alice:example.test".to_owned(),
                    ],
                    avatar: None,
                }]
            ),
            "expected UserProfilesUpdated action, got {actions:?}"
        );
    }

    #[tokio::test]
    async fn live_room_list_observer_projects_committed_invite_without_entry_diff() {
        use matrix_sdk::{
            ruma::{events::AnySyncStateEvent, room_id, serde::Raw, user_id},
            test_utils::mocks::MatrixMockServer,
        };
        use matrix_sdk_test::{
            InvitedRoomBuilder, JoinedRoomBuilder, LeftRoomBuilder, event_factory::EventFactory,
        };

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let visible_room_id = room_id!("!visible-room:example.invalid");
        let visible_room_name: Raw<AnySyncStateEvent> = EventFactory::new()
            .room(visible_room_id)
            .sender(user_id!("@sender:example.invalid"))
            .room_name("AAAA visible room")
            .into();
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(visible_room_id).add_state_event(visible_room_name),
            )
            .await;
        let room_updates_rx = client.subscribe_to_all_room_updates();
        let mut harness =
            spawn_live_observer_test_harness(client.clone(), server.uri(), 1, room_updates_rx)
                .await;

        let initial = harness.next_actions("initial RLS projection").await;
        assert!(initial.iter().any(
            |action| matches!(action, AppAction::InviteListUpdated { invites } if invites.is_empty())
        ));
        harness
            .expect_event(
                "initial RLS projection",
                LiveObserverTestEvent::RlsProjected {
                    wake_count: 1,
                    entries_len: 1,
                },
            )
            .await;

        let invited_room_id = room_id!("!invite-without-list-diff:example.invalid");
        let invited_room_name = EventFactory::new()
            .room(invited_room_id)
            .sender(user_id!("@sender:example.invalid"))
            .room_name("ZZZZ hidden invite");
        server
            .sync_room(
                &client,
                InvitedRoomBuilder::new(invited_room_id).add_state_event(invited_room_name),
            )
            .await;

        harness
            .expect_event(
                "invite base batch",
                LiveObserverTestEvent::BaseBatch {
                    wake_count: 1,
                    update_count: 1,
                    lagged: false,
                    projection_required: true,
                },
            )
            .await;

        let updated = harness.next_actions("committed invite projection").await;
        assert!(updated.iter().any(|action| {
            matches!(
                action,
                AppAction::InviteListUpdated { invites }
                    if invites.iter().any(|invite| invite.room_id == invited_room_id.as_str())
            )
        }));
        harness
            .expect_event(
                "invite base projection",
                LiveObserverTestEvent::BaseProjected {
                    wake_count: 1,
                    rls_wake_count: 1,
                    entries_len: 1,
                    action_delivered: true,
                },
            )
            .await;

        let renamed_invite = EventFactory::new()
            .room(invited_room_id)
            .sender(user_id!("@sender:example.invalid"))
            .room_name("ZZZZ renamed invite");
        server
            .sync_room(
                &client,
                InvitedRoomBuilder::new(invited_room_id).add_state_event(renamed_invite),
            )
            .await;
        harness
            .expect_event(
                "invite metadata base batch",
                LiveObserverTestEvent::BaseBatch {
                    wake_count: 2,
                    update_count: 1,
                    lagged: false,
                    projection_required: true,
                },
            )
            .await;
        let metadata_updated = harness.next_actions("invite metadata projection").await;
        assert!(metadata_updated.iter().any(|action| {
            matches!(
                action,
                AppAction::InviteListUpdated { invites }
                    if invites.iter().any(|invite| {
                        invite.room_id == invited_room_id.as_str()
                            && invite.display_name == "ZZZZ renamed invite"
                    })
            )
        }));
        harness
            .expect_event(
                "invite metadata base projection",
                LiveObserverTestEvent::BaseProjected {
                    wake_count: 2,
                    rls_wake_count: 1,
                    entries_len: 1,
                    action_delivered: true,
                },
            )
            .await;

        server
            .sync_room(&client, LeftRoomBuilder::new(invited_room_id))
            .await;
        harness
            .expect_event(
                "invite removal base batch",
                LiveObserverTestEvent::BaseBatch {
                    wake_count: 3,
                    update_count: 1,
                    lagged: false,
                    projection_required: true,
                },
            )
            .await;
        let removed = harness.next_actions("invite removal projection").await;
        assert!(removed.iter().any(
            |action| matches!(action, AppAction::InviteListUpdated { invites } if invites.is_empty())
        ));
        harness
            .expect_event(
                "invite removal base projection",
                LiveObserverTestEvent::BaseProjected {
                    wake_count: 3,
                    rls_wake_count: 1,
                    entries_len: 1,
                    action_delivered: true,
                },
            )
            .await;
        assert!(matches!(
            harness.action_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let hidden_joined_room_id = room_id!("!hidden-joined-room:example.invalid");
        let hidden_joined_room_name: Raw<AnySyncStateEvent> = EventFactory::new()
            .room(hidden_joined_room_id)
            .sender(user_id!("@sender:example.invalid"))
            .room_name("ZZZY hidden joined room")
            .into();
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(hidden_joined_room_id)
                    .add_state_event(hidden_joined_room_name),
            )
            .await;
        harness
            .expect_event(
                "ordinary joined base batch",
                LiveObserverTestEvent::BaseBatch {
                    wake_count: 4,
                    update_count: 1,
                    lagged: false,
                    projection_required: false,
                },
            )
            .await;
        assert!(matches!(
            harness.action_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        harness.stop().await;
    }

    #[tokio::test]
    async fn live_room_list_observer_reconciles_once_after_lagged_base_updates() {
        use matrix_sdk::test_utils::mocks::MatrixMockServer;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let (base_update_tx, base_update_rx) = broadcast::channel(1);
        let mut harness =
            spawn_live_observer_test_harness(client, server.uri(), 1, base_update_rx).await;

        harness.next_actions("initial empty RLS projection").await;
        harness
            .expect_event(
                "initial empty RLS projection",
                LiveObserverTestEvent::RlsProjected {
                    wake_count: 1,
                    entries_len: 0,
                },
            )
            .await;

        base_update_tx
            .send(matrix_sdk_base::sync::RoomUpdates::default())
            .expect("first base update");
        base_update_tx
            .send(matrix_sdk_base::sync::RoomUpdates::default())
            .expect("second base update should overrun capacity one");
        harness
            .expect_event(
                "lagged base batch",
                LiveObserverTestEvent::BaseBatch {
                    wake_count: 1,
                    update_count: 1,
                    lagged: true,
                    projection_required: true,
                },
            )
            .await;
        harness.next_actions("one lag self-heal projection").await;
        harness
            .expect_event(
                "lag self-heal projection",
                LiveObserverTestEvent::BaseProjected {
                    wake_count: 1,
                    rls_wake_count: 1,
                    entries_len: 0,
                    action_delivered: true,
                },
            )
            .await;

        base_update_tx
            .send(matrix_sdk_base::sync::RoomUpdates::default())
            .expect("post-lag fence update");
        harness
            .expect_event(
                "post-lag fence batch",
                LiveObserverTestEvent::BaseBatch {
                    wake_count: 2,
                    update_count: 1,
                    lagged: false,
                    projection_required: false,
                },
            )
            .await;
        assert!(matches!(
            harness.action_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        harness.stop().await;
    }

    #[tokio::test]
    async fn live_room_list_observer_keeps_entries_alive_after_base_receiver_closes() {
        use matrix_sdk::{ruma::room_id, test_utils::mocks::MatrixMockServer};
        use matrix_sdk_test::JoinedRoomBuilder;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let (base_update_tx, base_update_rx) = broadcast::channel(1);
        let mut harness =
            spawn_live_observer_test_harness(client.clone(), server.uri(), 2, base_update_rx).await;

        harness
            .next_actions("initial RLS projection before close")
            .await;
        harness
            .expect_event(
                "initial RLS projection before close",
                LiveObserverTestEvent::RlsProjected {
                    wake_count: 1,
                    entries_len: 0,
                },
            )
            .await;

        drop(base_update_tx);
        harness
            .expect_event("closed base receiver", LiveObserverTestEvent::BaseClosed)
            .await;

        let joined_room_id = room_id!("!joined-after-base-close:example.invalid");
        server
            .sync_room(&client, JoinedRoomBuilder::new(joined_room_id))
            .await;
        let actions = harness
            .next_actions("RLS projection after base close")
            .await;
        assert!(actions.iter().any(|action| {
            matches!(
                action,
                AppAction::RoomListUpdated { rooms, .. }
                    if rooms.iter().any(|room| room.room_id == joined_room_id.as_str())
            )
        }));
        harness
            .expect_event(
                "RLS projection after base close",
                LiveObserverTestEvent::RlsProjected {
                    wake_count: 2,
                    entries_len: 1,
                },
            )
            .await;

        harness.stop().await;
    }

    #[tokio::test]
    async fn project_room_list_snapshot_does_not_update_known_rooms_when_actions_are_undelivered() {
        let (action_tx, action_rx) = mpsc::channel(1);
        drop(action_rx);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let known_room_ids = Arc::new(RwLock::new(BTreeSet::new()));
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "Private room".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        project_room_list_snapshot(&snapshot, &known_room_ids, &action_tx, &event_tx).await;

        assert!(
            known_room_ids.read().expect("known rooms").is_empty(),
            "RoomActor known-room book must advance only after reducer projection delivery"
        );
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

    #[tokio::test]
    async fn reorder_spaces_projects_action() {
        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        handle
            .send(RoomMessage::Command(RoomCommand::ReorderSpaces {
                request_id: make_request_id(1),
                space_ids: vec![
                    "!space-b:example.test".to_owned(),
                    "!space-a:example.test".to_owned(),
                ],
            }))
            .await;

        let actions = action_rx.recv().await.expect("actions");
        assert!(
            matches!(
                actions.as_slice(),
                [AppAction::ReorderSpaces { space_ids }]
                    if space_ids == &vec![
                        "!space-b:example.test".to_owned(),
                        "!space-a:example.test".to_owned()
                    ]
            ),
            "expected ReorderSpaces action, got {actions:?}"
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
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags {
                    favourite: Some(MatrixRoomTagInfo {
                        order: Some("0.25".to_owned()),
                    }),
                    low_priority: None,
                },
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            }],
            invites: vec![],
            user_profiles: vec![],
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
                options: CreateRoomOptions {
                    name: "test room".to_owned(),
                    topic: None,
                    alias_localpart: None,
                    encrypted: false,
                    visibility: CreateRoomVisibility::Private,
                    parent_space: None,
                },
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
    fn room_actor_command_loop_never_awaits_room_list_refresh() {
        let source = include_str!("room.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");

        assert!(
            !production_source.contains("refresh_room_list().await"),
            "RoomActor command handling must not await room-list normalization; it can block user-visible operations under large room lists"
        );
    }

    #[test]
    fn legacy_room_list_observation_accepts_explicit_refresh_requests() {
        let source = include_str!("room.rs");
        let legacy_body = source
            .split("async fn run_legacy_room_list_observation")
            .nth(1)
            .expect("legacy observation function")
            .split("// ---------------------------------------------------------------------------")
            .next()
            .expect("legacy observation body");

        assert!(legacy_body.contains("mut refresh_rx: mpsc::Receiver<()>"));
        assert!(legacy_body.contains("refresh_rx.recv()"));
        assert!(legacy_body.contains("while refresh_rx.try_recv().is_ok()"));
    }

    #[test]
    fn sync_started_legacy_starts_observation_before_refresh_request() {
        let source = include_str!("room.rs");
        let sync_started_body = source
            .split("RoomMessage::SyncStarted")
            .nth(2)
            .expect("SyncStarted match arm")
            .split("RoomMessage::SyncStopped")
            .next()
            .expect("SyncStarted body");

        let start = sync_started_body
            .find("self.start_legacy_observation();")
            .expect("legacy observation starts");
        let refresh = sync_started_body
            .find("self.refresh_room_list();")
            .expect("legacy refresh request");

        assert!(
            start < refresh,
            "Legacy refresh must be requested through the observation loop after it starts"
        );
    }

    #[test]
    fn create_room_links_parent_space_child_with_created_room_id_before_completion_event() {
        let source = include_str!("room.rs");
        let create_body = source
            .split("async fn handle_create_room")
            .nth(1)
            .expect("create room handler")
            .split("async fn handle_create_public_directory_room")
            .next()
            .expect("create room body");

        let link = create_body
            .find("link_created_room_to_parent_space")
            .expect("create room should link parent space with the newly created room id");
        let completion_event = create_body
            .find("RoomEvent::RoomCreated")
            .expect("create room completion event");

        assert!(
            link < completion_event,
            "m.space.child must be sent using the SDK-created room id before Tauri observes RoomCreated"
        );

        let link_helper = source
            .split("async fn link_created_room_to_parent_space")
            .nth(1)
            .expect("created-room space link helper")
            .split("async fn handle_create_public_directory_room")
            .next()
            .expect("created-room space link helper body");
        assert!(
            !link_helper.contains("emit_failure"),
            "linking a created room into a parent space is best-effort; the room already exists, so it must not turn RoomCreated into a Tauri-visible failure"
        );
    }

    #[test]
    fn room_list_observation_relays_parent_only_space_links_before_projection() {
        let source = include_str!("room.rs");
        let live_body = source
            .split("async fn normalize_and_project_entries")
            .nth(1)
            .expect("live normalize helper")
            .split("async fn run_legacy_room_list_observation")
            .next()
            .expect("live normalize body");
        let legacy_body = source
            .split("async fn refresh_room_list_from_joined_rooms")
            .nth(1)
            .expect("legacy refresh helper")
            .split("async fn run_live_room_list_observation")
            .next()
            .expect("legacy refresh body");

        for body in [live_body, legacy_body] {
            let relay = body
                .find("relay_missing_space_child_links")
                .expect("room-list snapshots should relay missing m.space.child state");
            let projection = body
                .find("project_room_list_snapshot")
                .expect("room-list snapshot projection");
            assert!(
                relay < projection,
                "observation should relay missing links before projection without owning the mutation policy"
            );
            assert!(
                !body.contains("koushi_sdk::set_space_child"),
                "room-list observers must not perform server writes directly"
            );
        }
    }

    #[test]
    fn missing_space_child_repairs_are_actor_owned_and_retryable() {
        let source = include_str!("room.rs");
        let actor_body = source
            .split("async fn handle_missing_space_child_links")
            .nth(1)
            .expect("RoomActor should own missing space-child repair handling")
            .split("async fn stop_observation")
            .next()
            .expect("repair handler should precede observation teardown");

        assert!(
            source.contains("RoomMessage::MissingSpaceChildLinks"),
            "observation must relay missing links to the RoomActor mailbox"
        );
        assert!(
            actor_body.contains("classify_room_error(&error)"),
            "RoomActor-owned repair failures must be classified"
        );
        let success = actor_body
            .find("attempts.insert(key)")
            .expect("successful repair should record the dedupe key");
        let call = actor_body
            .find("koushi_sdk::set_space_child")
            .expect("RoomActor should perform the repair write");
        assert!(
            call < success,
            "dedupe key must be recorded only after set_space_child succeeds so transient failures remain retryable"
        );
    }

    #[test]
    fn room_list_projection_is_reliable_before_known_room_book_advances() {
        let source = include_str!("room.rs");
        let projection_body = source
            .split("async fn project_room_list_snapshot")
            .nth(1)
            .expect("room-list projection helper")
            .split("/// LegacySync-path refresh")
            .next()
            .expect("room-list projection body");
        let send = projection_body
            .find(".send(vec![")
            .expect("room-list projection must use reliable action delivery");
        let known = projection_body
            .find("replace_known_room_ids")
            .expect("room-list projection should update the actor known-room book");

        assert!(
            !projection_body.contains("try_send(vec!["),
            "room-list projection must not drop reducer snapshots under action-channel pressure"
        );
        assert!(
            send < known,
            "RoomActor known-room book must advance only after reducer projection delivery"
        );
    }

    #[test]
    fn directory_join_selects_room_before_room_joined_event_is_emitted() {
        let source = include_str!("room.rs");
        let join_body = source
            .split("async fn handle_join_directory_room")
            .nth(1)
            .expect("directory join handler")
            .split("async fn handle_mark_room_as_read")
            .next()
            .expect("directory join body");
        let success_reduce = join_body
            .find("AppAction::DirectoryJoinSucceeded")
            .expect("directory join success reduction");
        let joined_event = join_body
            .find("RoomEvent::RoomJoined")
            .expect("directory join completion event");

        assert!(
            success_reduce < joined_event,
            "DirectoryJoinSucceeded must select the room before Tauri observes RoomJoined"
        );
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
            .find("self.reduce_reliable(vec![AppAction::PinEventCompleted")
            .expect("pin completion action");
        let pin_reload = pin_body
            .find("project_pinned_events_after_success")
            .expect("pin projection reload");
        assert!(pin_completion < pin_reload);

        let unpin_completion = unpin_body
            .find("self.reduce_reliable(vec![AppAction::UnpinEventCompleted")
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
            .find("koushi_sdk::pin_event")
            .expect("pin sdk call");
        assert!(pin_guard < pin_sdk);

        let unpin_guard = unpin_body
            .find("ensure_known_room_for_message_interaction")
            .expect("unpin known-room guard");
        let unpin_sdk = unpin_body
            .find("koushi_sdk::unpin_event")
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
    async fn session_lifecycle_messages_without_session_complete_cleanly() {
        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, _event_rx) = broadcast::channel(16);
        let handle = RoomActor::spawn(action_tx, event_tx);

        // No session, no observation loop: both must be no-ops, and the
        // actor task must still exit on Shutdown.
        assert!(handle.send(RoomMessage::SyncStopped).await);
        assert!(handle.send(RoomMessage::SessionCleared).await);
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
