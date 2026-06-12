//! RoomActor: room list normalization and room operations.
//!
//! ## Ownership
//! `RoomActor` is owned by `AccountActor`. Its task handle lives inside
//! `AccountActor`; colocated as a child task per the spec
//! ("Actor Deployment And Supervision — boundaries define ownership, not one
//! task per actor").
//!
//! ## Room list normalization (Async rule 9 note)
//! On `RoomMessage::SyncStarted`, `RoomActor` does an initial
//! `matrix_desktop_auth::room_list_snapshot(session)` call — which internally
//! tries `RoomListService` (SyncService backend) and falls back to
//! `client.joined_rooms()` (LegacySync backend) — and then spawns a room-list
//! observation loop subscribed to
//! `client.subscribe_to_all_room_updates()`. That broadcast fires on both
//! backends because both feed the base client, so the actor RELAYS the SDK's
//! observable stream (Async rule 1) instead of taking a one-shot snapshot.
//! Each received update batch coalesces any additionally pending batches
//! (`try_recv` drain) into a single re-normalization; a `Lagged` receiver
//! triggers one refresh because the snapshot is self-healing. Snapshots are
//! projected as `AppAction::RoomListUpdated` + `RoomEvent::RoomListUpdated`.
//!
//! The auth snapshot function handles both backend paths so `RoomActor` does
//! not need a direct reference to the `SyncService` that `SyncActor` holds.
//!
//! Per Async rule 9: "Because the local QA matrix includes homeservers without
//! MSC4186, this legacy room-list path is a fully implemented, QA-gated
//! product path, not a stub."
//!
//! ## Room operations
//! `CreateRoom`, `CreateSpace`, `SetSpaceChild`, `InviteUser`, `JoinRoom` call
//! `matrix-desktop-auth` primitives and emit domain events with `request_id`.
//! Errors are classified into `RoomFailureKind` (never raw SDK text).
//!
//! ## SelectSpace / SelectRoom
//! Pure navigation — project `AppAction::SelectSpace` / `AppAction::SelectRoom`
//! through the action channel. The reducer may return `AppEffect::SubscribeTimeline`
//! for `SelectRoom` — effects are currently dropped by `AppActor`. That is
//! Phase 5's job; see the comment in `AppActor::run()`.
//!
//! ## Security
//! Raw SDK error text never appears in events or AppState. All errors are
//! classified into `RoomFailureKind`.

use std::sync::Arc;

use matrix_desktop_auth::{MatrixClientSession, MatrixRoomOperationError};
use matrix_desktop_state::{AppAction, RoomSummary, SpaceSummary};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::command::RoomCommand;
use crate::event::{CoreEvent, RoomEvent};
use crate::executor;
use crate::failure::{CoreFailure, RoomFailureKind};
use crate::ids::RequestId;

/// Messages sent to the RoomActor from AccountActor.
pub enum RoomMessage {
    /// Route a `RoomCommand` to the actor.
    Command(RoomCommand),
    /// Sync started: the room actor should refresh its room list now.
    /// `AccountActor` sends this after `SyncEvent::Started` is observed,
    /// supplying the store-backed session so room ops are available.
    SyncStarted {
        session: Arc<MatrixClientSession>,
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
/// `sync.rs` `legacy_stop_tx`).
struct RoomListObservation {
    stop_tx: oneshot::Sender<()>,
    task: executor::JoinHandle<()>,
}

pub struct RoomActor {
    session: Option<Arc<MatrixClientSession>>,
    observation: Option<RoomListObservation>,
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
                RoomMessage::SyncStarted { session } => {
                    // Guard against two observation loops running: a previous
                    // loop (from an earlier SyncStarted) is stopped before the
                    // replacement is spawned.
                    self.stop_observation().await;
                    self.session = Some(session);
                    // Initial snapshot to populate state, then a continuous
                    // observation loop so later room updates keep relaying
                    // (Async rule 1: actors relay the SDK's observable
                    // streams; a one-shot snapshot is not relaying).
                    self.refresh_room_list().await;
                    self.start_observation();
                }
                RoomMessage::SyncStopped => {
                    self.stop_observation().await;
                }
            }
        }
    }

    /// Spawn the room-list observation loop for the current session.
    fn start_observation(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let task = executor::spawn(run_room_list_observation(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            stop_rx,
        ));
        self.observation = Some(RoomListObservation { stop_tx, task });
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
            RoomCommand::CreateRoom { request_id, name } => {
                self.handle_create_room(request_id, name).await;
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
            RoomCommand::JoinRoom {
                request_id,
                room_id,
            } => {
                self.handle_join_room(request_id, room_id).await;
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
                // The reducer may return AppEffect::SubscribeTimeline — effects
                // are dropped by AppActor until Phase 5 implements TimelineActor.
                // TODO(phase-5): handle AppEffect::SubscribeTimeline here.
                self.reduce(vec![AppAction::SelectRoom { room_id }]);
            }
        }
    }

    async fn handle_create_room(&self, request_id: RequestId, name: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_auth::create_room(session, &name).await {
            Ok(room_id) => {
                self.emit(CoreEvent::Room(RoomEvent::RoomCreated {
                    request_id,
                    room_id,
                }));
                // Reflect the actor's own mutation immediately instead of
                // waiting for the next sync round-trip.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed { kind },
                );
            }
        }
    }

    async fn handle_create_space(&self, request_id: RequestId, name: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_auth::create_space(session, &name).await {
            Ok(space_id) => {
                self.emit(CoreEvent::Room(RoomEvent::SpaceCreated {
                    request_id,
                    space_id,
                }));
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed { kind },
                );
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
        match matrix_desktop_auth::set_space_child(session, &space_id, &child_room_id, &via_server)
            .await
        {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::SpaceChildSet {
                    request_id,
                    space_id,
                    child_room_id,
                }));
                // Reflect the actor's own mutation immediately.
                self.refresh_room_list().await;
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed { kind },
                );
            }
        }
    }

    async fn handle_invite_user(
        &self,
        request_id: RequestId,
        room_id: String,
        user_id: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_auth::invite_user_to_room(session, &room_id, &user_id).await {
            Ok(()) => {
                self.emit(CoreEvent::Room(RoomEvent::UserInvited {
                    request_id,
                    room_id,
                    user_id,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed { kind },
                );
            }
        }
    }

    async fn handle_join_room(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match matrix_desktop_auth::join_room_by_id(session, &room_id).await {
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
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed { kind },
                );
            }
        }
    }

    /// Fetch the current room list and project it into AppState via the action
    /// channel. Also emits `RoomEvent::RoomListUpdated` as a discrete event.
    async fn refresh_room_list(&self) {
        if let Some(session) = &self.session {
            refresh_room_list_with(session, &self.action_tx, &self.event_tx).await;
        }
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

/// Fetch the current room list, normalize it, and project it as
/// `AppAction::RoomListUpdated` + `RoomEvent::RoomListUpdated`.
///
/// `matrix_desktop_auth::room_list_snapshot` handles both the SyncService
/// backend (tries RoomListService first) and the LegacySync backend (falls
/// back to client.joined_rooms()), so this single call covers both QA-gated
/// paths per Async rule 9.
async fn refresh_room_list_with(
    session: &MatrixClientSession,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
) {
    let snapshot = match matrix_desktop_auth::room_list_snapshot(session).await {
        Ok(s) => s,
        Err(_) => return,
    };

    let spaces = normalize_spaces(&snapshot);
    let rooms = normalize_rooms(&snapshot);

    let _ = action_tx.try_send(vec![AppAction::RoomListUpdated { spaces, rooms }]);
    let _ = event_tx.send(CoreEvent::Room(RoomEvent::RoomListUpdated));
}

/// Room-list observation loop (Async rule 1: relay the SDK's observable
/// streams). Subscribes to `client.subscribe_to_all_room_updates()`, which
/// fires on both SyncService and LegacySync backends because both feed the
/// base client. Each received batch coalesces any additionally pending
/// batches into one `refresh_room_list_with` call; `Lagged` triggers a single
/// refresh because the snapshot is self-healing. Exits on the oneshot stop
/// signal (same pattern as `sync.rs` `legacy_stop_tx`) or when the SDK closes
/// the broadcast.
async fn run_room_list_observation(
    session: Arc<MatrixClientSession>,
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
                    refresh_room_list_with(&session, &action_tx, &event_tx).await;
                }
                Err(RecvError::Lagged(_)) => {
                    // The snapshot is self-healing: refresh once.
                    refresh_room_list_with(&session, &action_tx, &event_tx).await;
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
fn normalize_spaces(
    snapshot: &matrix_desktop_auth::MatrixRoomListSnapshot,
) -> Vec<SpaceSummary> {
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
                child_room_ids,
            }
        })
        .collect()
}

/// Convert `MatrixRoomListSnapshot` rooms into `RoomSummary` values.
fn normalize_rooms(
    snapshot: &matrix_desktop_auth::MatrixRoomListSnapshot,
) -> Vec<RoomSummary> {
    snapshot
        .rooms
        .iter()
        .map(|room| RoomSummary {
            room_id: room.room_id.clone(),
            display_name: room.display_name.clone(),
            is_dm: room.is_dm,
            unread_count: room.unread_count,
            parent_space_ids: room.parent_space_ids.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Error classification (never raw SDK text in public events)
// ---------------------------------------------------------------------------

/// Map a `MatrixRoomOperationError` to a coarse `RoomFailureKind`.
/// The spec defines: Forbidden / NotFound / Network / Sdk.
/// Raw SDK error text must never appear in public events.
pub(crate) fn classify_room_error(error: &MatrixRoomOperationError) -> RoomFailureKind {
    use matrix_desktop_auth::MatrixRoomOperationFailureKind;
    match error {
        MatrixRoomOperationError::InvalidRoomId
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
    use matrix_desktop_auth::{MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace};
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
            matrix_desktop_auth::MatrixRoomOperationFailureKind::Forbidden,
        );
        assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
    }

    #[test]
    fn auth_required_sdk_error_classifies_as_forbidden() {
        let error = MatrixRoomOperationError::Sdk(
            matrix_desktop_auth::MatrixRoomOperationFailureKind::AuthenticationRequired,
        );
        assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
    }

    #[test]
    fn http_sdk_error_classifies_as_network() {
        let error = MatrixRoomOperationError::Sdk(
            matrix_desktop_auth::MatrixRoomOperationFailureKind::Http,
        );
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
        let error = MatrixRoomOperationError::Sdk(
            matrix_desktop_auth::MatrixRoomOperationFailureKind::Sdk,
        );
        assert_eq!(classify_room_error(&error), RoomFailureKind::Sdk);
    }

    // --- Room list normalization: spaces ---

    #[test]
    fn normalize_spaces_with_child_rooms() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space1:example.test".to_owned(),
                display_name: "My Space".to_owned(),
            }],
            rooms: vec![
                MatrixRoomListRoom {
                    room_id: "!room1:example.test".to_owned(),
                    display_name: "Room 1".to_owned(),
                    is_dm: false,
                    unread_count: 0,
                    parent_space_ids: vec!["!space1:example.test".to_owned()],
                },
                MatrixRoomListRoom {
                    room_id: "!room2:example.test".to_owned(),
                    display_name: "Room 2".to_owned(),
                    is_dm: false,
                    unread_count: 0,
                    parent_space_ids: vec![],
                },
            ],
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
            }],
            rooms: vec![],
        };
        let spaces = normalize_spaces(&snapshot);
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].child_room_ids, Vec::<String>::new());
    }

    // --- Room list normalization: rooms ---

    #[test]
    fn normalize_rooms_preserves_dm_and_unread() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!dm:example.test".to_owned(),
                display_name: "Alice".to_owned(),
                is_dm: true,
                unread_count: 3,
                parent_space_ids: vec![],
            }],
        };
        let rooms = normalize_rooms(&snapshot);
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_id, "!dm:example.test");
        assert!(rooms[0].is_dm);
        assert_eq!(rooms[0].unread_count, 3);
    }

    #[test]
    fn normalize_rooms_non_dm() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                is_dm: false,
                unread_count: 0,
                parent_space_ids: vec!["!space:example.test".to_owned()],
            }],
        };
        let rooms = normalize_rooms(&snapshot);
        assert_eq!(rooms.len(), 1);
        assert!(!rooms[0].is_dm);
        assert_eq!(rooms[0].parent_space_ids, vec!["!space:example.test"]);
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
            }))
            .await;

        let event = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            event_rx.recv(),
        )
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
