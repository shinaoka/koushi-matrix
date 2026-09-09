use super::actor::{MissingSpaceChildLink, RoomActor};
use super::list_observer::{record_residency_ack_failure, record_residency_admission_failure};
use crate::timeline::{
    RoomRemovalCause, TimelineSubscriptionResidencyHandle, TimelineSubscriptionResidencyPermit,
};
use crate::unread_trace;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::command::{CreateRoomOptions, CreateRoomVisibility};
use koushi_protocol::event::{CoreEvent, ReportKind, RoomEvent};
use koushi_protocol::failure::{CoreFailure, RoomFailureKind};
use koushi_protocol::ids::RequestId;
use koushi_sdk::{
    MatrixClientSession, MatrixCreateRoomOptions, MatrixCreateRoomParentSpace,
    MatrixCreateRoomVisibility, MatrixRoomOperationError, MatrixRoomTagKind,
};
use koushi_state::{
    AppAction, BasicOperationRequest, INVITE_ALREADY_IN_SPACE_MESSAGE, InviteDestination,
    InviteDestinationResult, InviteDestinationResultKind, InviteScopeSelection,
    OperationFailureKind, RoomNotificationMode, RoomTagInfo, RoomTagKind,
};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::Mutex;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::Ordering;
use std::{future::Future, sync::Arc};
#[cfg(any(test, feature = "test-hooks"))]
use tokio::sync::oneshot;

/// Fixed, content-free messages recorded in `AppState.errors` when a basic
/// operation fails. Raw SDK errors are classified into `RoomFailureKind` for the
/// transport `OperationFailed` event and never placed in product state.
const CREATE_ROOM_FAILED_MESSAGE: &str = "Room creation failed";

const CREATE_SPACE_FAILED_MESSAGE: &str = "Space creation failed";

const LINK_SPACE_CHILD_FAILED_MESSAGE: &str = "Linking the room to the space failed";

pub(super) type SpaceChildLinkKey = (String, String);

/// Closed membership-operation kinds used by the test-only result seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoomOperationKind {
    LeaveRoom,
    DeclineInvite,
    AcceptInvite,
    JoinRoom,
    JoinDirectoryRoom,
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) struct RoomOperationTestControl {
    pub(crate) kind: RoomOperationKind,
    pub(crate) reached: oneshot::Sender<()>,
    pub(crate) completion: oneshot::Receiver<Result<String, MatrixRoomOperationError>>,
}

#[cfg(any(test, feature = "test-hooks"))]
fn take_matching_room_operation_test_control(
    control: &mut Option<RoomOperationTestControl>,
    kind: RoomOperationKind,
) -> Option<RoomOperationTestControl> {
    if control.as_ref().is_some_and(|control| control.kind == kind) {
        control.take()
    } else {
        None
    }
}

#[cfg(any(test, feature = "test-hooks"))]
pub(super) type RoomOperationTestControlSlot = Arc<Mutex<Option<RoomOperationTestControl>>>;

#[cfg(test)]
#[test]
fn room_operation_test_control_matches_and_consumes_once() {
    let (reached, _reached_rx) = oneshot::channel();
    let (_completion, completion_rx) = oneshot::channel();
    let mut control = Some(RoomOperationTestControl {
        kind: RoomOperationKind::LeaveRoom,
        reached,
        completion: completion_rx,
    });

    assert!(
        take_matching_room_operation_test_control(&mut control, RoomOperationKind::JoinRoom,)
            .is_none()
    );
    assert!(control.is_some());
    assert!(
        take_matching_room_operation_test_control(&mut control, RoomOperationKind::LeaveRoom,)
            .is_some()
    );
    assert!(
        take_matching_room_operation_test_control(&mut control, RoomOperationKind::LeaveRoom,)
            .is_none()
    );
}

pub(super) struct AdmittedRoomOperation {
    pub(super) session: Arc<MatrixClientSession>,
    pub(super) residency: TimelineSubscriptionResidencyHandle,
    pub(super) permit: TimelineSubscriptionResidencyPermit,
}

impl AdmittedRoomOperation {
    pub(super) async fn room_left(&self, room_id: &str, cause: RoomRemovalCause) -> bool {
        let Ok(room_id) = room_id.parse() else {
            return false;
        };
        self.residency.room_left(&self.permit, room_id, cause).await
    }

    pub(super) async fn room_rejoined(&self, room_id: &str) -> bool {
        let Ok(room_id) = room_id.parse() else {
            return false;
        };
        self.residency.room_rejoined(&self.permit, room_id).await
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

pub(super) fn operation_failure_kind(kind: RoomFailureKind) -> OperationFailureKind {
    match kind {
        RoomFailureKind::AliasInUse => OperationFailureKind::Invalid,
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

/// Map a `MatrixRoomOperationError` to a coarse `RoomFailureKind`.
/// Alias collisions retain their actionable kind; raw error details stay private.
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
            MatrixRoomOperationFailureKind::AliasInUse => RoomFailureKind::AliasInUse,
            MatrixRoomOperationFailureKind::Forbidden
            | MatrixRoomOperationFailureKind::AuthenticationRequired => RoomFailureKind::Forbidden,
            MatrixRoomOperationFailureKind::Http => RoomFailureKind::Network,
            MatrixRoomOperationFailureKind::Sdk
            | MatrixRoomOperationFailureKind::Encryption
            | MatrixRoomOperationFailureKind::Store
            | MatrixRoomOperationFailureKind::SecureBackupRequired
            | MatrixRoomOperationFailureKind::WrongRoomState => RoomFailureKind::Sdk,
        },
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

impl RoomActor {
    pub(super) async fn handle_missing_space_child_links(
        &mut self,
        links: Vec<MissingSpaceChildLink>,
    ) {
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

    pub(super) async fn handle_create_room(
        &self,
        request_id: RequestId,
        options: CreateRoomOptions,
    ) {
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
        parent_space: Option<&koushi_protocol::command::CreateRoomParentSpace>,
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

    pub(super) async fn handle_create_space(&self, request_id: RequestId, name: String) {
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

    pub(super) async fn handle_set_space_child(
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

    pub(super) async fn handle_invite_user(
        &self,
        request_id: RequestId,
        room_id: String,
        user_id: String,
    ) {
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

    pub(super) async fn handle_invite_targets(
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

    pub(super) fn begin_residency_operation(&self) -> Result<AdmittedRoomOperation, ()> {
        let Some(session) = self.session.as_ref() else {
            return Err(());
        };
        let Some(binding) = self.timeline_residency.borrow().clone() else {
            record_residency_admission_failure("binding_missing");
            return Err(());
        };
        if !Arc::ptr_eq(session, &binding.session) {
            record_residency_admission_failure("session_mismatch");
            return Err(());
        }
        let Some(permit) = binding.handle.begin_operation() else {
            record_residency_admission_failure("manager_closed");
            return Err(());
        };
        Ok(AdmittedRoomOperation {
            session: binding.session,
            residency: binding.handle,
            permit,
        })
    }

    pub(super) fn reject_residency_operation(&self, request_id: RequestId) {
        self.emit_failure(
            request_id,
            CoreFailure::RoomOperationFailed {
                kind: RoomFailureKind::Sdk,
            },
        );
    }

    pub(super) fn reject_residency_ack(&self, request_id: RequestId) {
        record_residency_ack_failure();
        self.emit_failure(
            request_id,
            CoreFailure::RoomOperationFailed {
                kind: RoomFailureKind::Sdk,
            },
        );
    }

    pub(super) async fn call_room_operation<F>(
        &self,
        kind: RoomOperationKind,
        real_call: F,
    ) -> Result<String, MatrixRoomOperationError>
    where
        F: Future<Output = Result<String, MatrixRoomOperationError>>,
    {
        #[cfg(any(test, feature = "test-hooks"))]
        self.room_operation_test_reached_count
            .fetch_add(1, Ordering::SeqCst);
        #[cfg(any(test, feature = "test-hooks"))]
        let control = take_matching_room_operation_test_control(
            &mut *self
                .room_operation_test_control
                .lock()
                .expect("room operation test control lock"),
            kind,
        );
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(control) = control {
            let _ = control.reached.send(());
            return match control.completion.await {
                Ok(result) => result,
                Err(_) => Err(koushi_sdk::MatrixRoomOperationError::Sdk(
                    koushi_sdk::MatrixRoomOperationFailureKind::Sdk,
                )),
            };
        }

        let _ = kind;
        real_call.await
    }

    async fn leave_room_with_residency(
        &self,
        kind: RoomOperationKind,
        room_id: &str,
    ) -> Option<(
        AdmittedRoomOperation,
        Result<String, koushi_sdk::MatrixRoomOperationError>,
    )> {
        let operation = self.begin_residency_operation().ok()?;
        let result = self
            .call_room_operation(kind, koushi_sdk::leave_room(&operation.session, room_id))
            .await;
        Some((operation, result))
    }

    pub(super) async fn handle_accept_invite(&self, request_id: RequestId, room_id: String) {
        let Some(_session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let operation = match self.begin_residency_operation() {
            Ok(operation) => operation,
            Err(()) => {
                self.reject_residency_operation(request_id);
                return;
            }
        };
        match self
            .call_room_operation(
                RoomOperationKind::AcceptInvite,
                koushi_sdk::join_room_by_id(&operation.session, &room_id),
            )
            .await
        {
            Ok(joined_room_id) => {
                if !operation.room_rejoined(&joined_room_id).await {
                    self.reject_residency_ack(request_id);
                    return;
                }
                koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.room_operation",
                    "accept_join_returned",
                ));
                self.refresh_room_list_for_room(&joined_room_id);
                koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.room_operation",
                    "accept_event_emit_started",
                ));
                self.emit(CoreEvent::Room(RoomEvent::InviteAccepted {
                    request_id,
                    room_id: joined_room_id,
                }));
                koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.room_operation",
                    "accept_event_emit_completed",
                ));
                self.refresh_room_list();
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_decline_invite(&self, request_id: RequestId, room_id: String) {
        let Some(_session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let Some((operation, result)) = self
            .leave_room_with_residency(RoomOperationKind::DeclineInvite, &room_id)
            .await
        else {
            self.reject_residency_operation(request_id);
            return;
        };
        match result {
            Ok(declined_room_id) => {
                if !operation
                    .room_left(&declined_room_id, RoomRemovalCause::InviteDecline)
                    .await
                {
                    self.reject_residency_ack(request_id);
                    return;
                }
                self.refresh_room_list();
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

    pub(super) async fn handle_start_direct_message(&self, request_id: RequestId, user_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        match koushi_sdk::start_direct_message(session, &user_id).await {
            Ok(room_id) => {
                koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.room_operation",
                    "start_dm_returned",
                ));
                let room_id_for_projection = room_id.clone();
                self.emit(CoreEvent::Room(RoomEvent::DirectMessageStarted {
                    request_id,
                    room_id,
                }));
                koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.room_operation",
                    "start_dm_event_emit_completed",
                ));
                self.refresh_room_list_for_room(&room_id_for_projection);
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_join_room(&self, request_id: RequestId, room_id: String) {
        let Some(_session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let operation = match self.begin_residency_operation() {
            Ok(operation) => operation,
            Err(()) => {
                self.reject_residency_operation(request_id);
                return;
            }
        };
        match self
            .call_room_operation(
                RoomOperationKind::JoinRoom,
                koushi_sdk::join_room_by_id(&operation.session, &room_id),
            )
            .await
        {
            Ok(joined_room_id) => {
                if !operation.room_rejoined(&joined_room_id).await {
                    self.reject_residency_ack(request_id);
                    return;
                }
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

    pub(super) async fn handle_leave_room(&mut self, request_id: RequestId, room_id: String) {
        let Some(_session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let Some((operation, result)) = self
            .leave_room_with_residency(RoomOperationKind::LeaveRoom, &room_id)
            .await
        else {
            self.reject_residency_operation(request_id);
            return;
        };
        match result {
            Ok(left_room_id) => {
                if !operation
                    .room_left(&left_room_id, RoomRemovalCause::DirectLeave)
                    .await
                {
                    self.reject_residency_ack(request_id);
                    return;
                }
                self.reduce_reliable(vec![AppAction::SpaceOrderPreferenceRemoved {
                    space_id: left_room_id.clone(),
                }])
                .await;
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

    pub(super) async fn handle_forget_room(&self, request_id: RequestId, room_id: String) {
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

    pub(super) async fn handle_set_tag(
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

    pub(super) async fn handle_remove_tag(
        &self,
        request_id: RequestId,
        room_id: String,
        tag: RoomTagKind,
    ) {
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

    pub(super) async fn handle_mark_room_as_read(
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

    pub(super) async fn handle_mark_room_as_unread(
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

    pub(super) async fn handle_force_rotate_outbound_session(
        &self,
        request_id: RequestId,
        room_id: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        if !self.ensure_known_room_for_message_interaction(request_id, &room_id) {
            return;
        }
        match koushi_sdk::discard_outbound_room_key(session, &room_id).await {
            Ok(()) => self.emit(CoreEvent::Room(RoomEvent::OutboundSessionRotationForced {
                request_id,
                room_id,
            })),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_set_room_notification_mode(
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

    pub(super) async fn handle_report_content(
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
                        kind: crate::report::classify_report_error(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_report_room(
        &self,
        request_id: RequestId,
        room_id: String,
        reason: String,
    ) {
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
                        kind: crate::report::classify_report_error(&error),
                    },
                );
            }
        }
    }

    pub(super) fn clear_space_child_repair_attempts(&self) {
        if let Ok(mut attempts) = self.attempted_space_child_repairs.write() {
            attempts.clear();
        }
    }

    fn mark_space_child_link_attempted(&self, space_id: &str, child_room_id: &str) {
        if let Ok(mut attempts) = self.attempted_space_child_repairs.write() {
            attempts.insert((space_id.to_owned(), child_room_id.to_owned()));
        }
    }

    pub(super) fn ensure_known_room_for_message_interaction(
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
}

#[cfg(test)]
mod tests;
