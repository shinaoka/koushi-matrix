//! Issue #961: loading a Space's advertised children.
//!
//! Mirrors the Space member load: the command is admitted against the selected
//! Space and its generation, the SDK projection runs, and the result is
//! reduced under the same generation so a slow response cannot paint another
//! Space's rooms.
//!
//! Issue #1410 adds the `suggested` flag: its admin toggle and the one-time
//! bulk join of a newly joined Space's suggested rooms.

use koushi_protocol::event::{CoreEvent, RoomEvent};
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::RequestId;
use koushi_state::{
    AppAction, OperationFailureKind, SpaceChildMembership, SpaceChildSummary,
    SuggestedRoomJoinOutcome,
};

use super::RoomActor;
use super::normalization::avatar_from_mxc_uri;
use super::operations::{RoomOperationKind, classify_room_error, operation_failure_kind};
use koushi_sdk::{MatrixSpaceChildEntry, MatrixSpaceChildMembership};

impl RoomActor {
    pub(super) async fn handle_load_space_children(
        &mut self,
        request_id: RequestId,
        space_id: String,
        generation: u64,
    ) {
        let Some(session) = self.session.clone() else {
            self.reduce_reliable(vec![AppAction::SpaceChildrenLoadFailed {
                space_id,
                generation,
                failure: OperationFailureKind::Sdk,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::matrix_space_children_projection(&session, &space_id).await {
            Ok(entries) => {
                let children: Vec<SpaceChildSummary> =
                    entries.iter().map(space_child_summary).collect();
                let child_count = children.len();
                // #1410: an unreadable permission hides the toggle rather than
                // failing the load.
                let can_edit_suggested =
                    koushi_sdk::matrix_space_can_edit_children(&session, &space_id)
                        .await
                        .unwrap_or(false);
                self.reduce_reliable(vec![AppAction::SpaceChildrenLoaded {
                    space_id,
                    generation,
                    children,
                    can_edit_suggested,
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::SpaceChildrenLoaded {
                    request_id,
                    generation,
                    child_count,
                }));
            }
            Err(error) => {
                let failure = OperationFailureKind::Sdk;
                self.reduce_reliable(vec![AppAction::SpaceChildrenLoadFailed {
                    space_id,
                    generation,
                    failure,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed {
                        kind: classify_room_error(&error),
                    },
                );
            }
        }
    }

    /// Issue #1410: write the admitted `suggested` flag and settle the toggle.
    pub(super) async fn handle_set_space_child_suggested(
        &self,
        request_id: RequestId,
        space_id: String,
        child_room_id: String,
        suggested: bool,
    ) {
        let result = match self.session.as_ref() {
            Some(session) => {
                koushi_sdk::set_space_child_suggested(session, &space_id, &child_room_id, suggested)
                    .await
                    .map_err(|error| CoreFailure::RoomOperationFailed {
                        kind: classify_room_error(&error),
                    })
            }
            None => Err(CoreFailure::SessionRequired),
        };
        let failure = match &result {
            Ok(_) => None,
            Err(CoreFailure::RoomOperationFailed { kind }) => Some(operation_failure_kind(*kind)),
            Err(_) => Some(OperationFailureKind::Sdk),
        };
        self.reduce_reliable(vec![AppAction::SpaceChildSuggestedUpdateSettled {
            request_id: request_id.sequence,
            space_id,
            child_room_id,
            suggested,
            failure,
        }])
        .await;
        if let Err(failure) = result {
            self.emit_failure(request_id, failure);
        }
    }

    /// Issue #1410: join the admitted suggested rooms one at a time and report
    /// each outcome. A failure never stops the remaining joins, and nothing
    /// here changes a join rule or sends an invitation.
    pub(super) async fn handle_join_suggested_space_rooms(
        &self,
        request_id: RequestId,
        space_id: String,
        room_ids: Vec<String>,
    ) {
        let mut outcomes = Vec::with_capacity(room_ids.len());
        match self.begin_residency_operation() {
            Ok(operation) => {
                for room_id in room_ids {
                    let failure = match self
                        .call_room_operation(
                            RoomOperationKind::JoinRoom,
                            koushi_sdk::join_room_by_id(&operation.session, &room_id),
                        )
                        .await
                    {
                        Ok(joined_room_id) => {
                            if operation.room_rejoined(&joined_room_id).await {
                                self.reduce_reliable(vec![AppAction::RoomJoinedLocally {
                                    room_id: joined_room_id,
                                }])
                                .await;
                                None
                            } else {
                                Some(OperationFailureKind::Sdk)
                            }
                        }
                        Err(error) => Some(operation_failure_kind(classify_room_error(&error))),
                    };
                    outcomes.push(SuggestedRoomJoinOutcome { room_id, failure });
                }
                self.reduce_reliable(vec![AppAction::SuggestedRoomsJoinSettled {
                    request_id: request_id.sequence,
                    space_id,
                    outcomes,
                }])
                .await;
                drop(operation);
                self.refresh_room_list();
            }
            Err(()) => {
                // No outcome reported: every offered room settles as failed.
                self.reduce_reliable(vec![AppAction::SuggestedRoomsJoinSettled {
                    request_id: request_id.sequence,
                    space_id,
                    outcomes,
                }])
                .await;
                self.reject_residency_operation(request_id);
            }
        }
    }
}

fn space_child_summary(entry: &MatrixSpaceChildEntry) -> SpaceChildSummary {
    SpaceChildSummary {
        room_id: entry.room_id.clone(),
        display_name: entry.display_name.clone(),
        display_name_placeholder: entry.display_name_placeholder.clone(),
        avatar: avatar_from_mxc_uri(entry.avatar_mxc_uri.as_deref()),
        topic: entry.topic.clone(),
        membership: match entry.membership {
            MatrixSpaceChildMembership::Joined => SpaceChildMembership::Joined,
            MatrixSpaceChildMembership::Invited => SpaceChildMembership::Invited,
            MatrixSpaceChildMembership::Knocked => SpaceChildMembership::Knocked,
            MatrixSpaceChildMembership::Left => SpaceChildMembership::Left,
            MatrixSpaceChildMembership::Banned => SpaceChildMembership::Banned,
            MatrixSpaceChildMembership::NotJoined => SpaceChildMembership::NotJoined,
            MatrixSpaceChildMembership::Unknown => SpaceChildMembership::Unknown,
        },
        can_join: entry.can_join,
        suggested: entry.suggested,
        is_space: entry.is_space,
        joined_members: entry.joined_members,
    }
}
