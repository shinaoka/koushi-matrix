use crate::composer_draft_lifecycle::ComposerDraftScope;
use crate::native_artifact::NativeArtifactKind;
use koushi_protocol::command::{
    AccountCommand, AppCommand, CoreCommand, SearchScope, TimelineCommand,
};
use koushi_protocol::ids::{RequestId, TimelineKind};
use koushi_state::{AppAction, AppState, OperationFailureKind, admit_space_leave_room_ids};

/// Protocol message accompanying a failed search, shared by the routing layer
/// and the search actor so both settle the UI the same way.
pub(crate) const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";

/// Narrow a `LeaveSpace` command's child rooms to the Space's current leave
/// candidates, so a stale or forged ID never leaves a room outside the Space.
/// A `JoinSuggestedSpaceRooms` command joins exactly the rooms of the admitted
/// offer (#1410), whatever the caller sent. Every other command passes through
/// unchanged.
pub(crate) fn admit_leave_space_command(
    state: &AppState,
    command: koushi_protocol::command::RoomCommand,
) -> koushi_protocol::command::RoomCommand {
    match command {
        koushi_protocol::command::RoomCommand::LeaveSpace {
            request_id,
            space_id,
            child_room_ids,
        } => {
            let child_room_ids = admit_space_leave_room_ids(state, &space_id, &child_room_ids);
            koushi_protocol::command::RoomCommand::LeaveSpace {
                request_id,
                space_id,
                child_room_ids,
            }
        }
        koushi_protocol::command::RoomCommand::JoinSuggestedSpaceRooms {
            request_id,
            space_id,
            room_ids: _,
        } => {
            let room_ids = match &state.space_children.suggested_offer {
                koushi_state::SuggestedRoomsOffer::Joining {
                    request_id: admitted,
                    space_id: offered,
                    rooms,
                } if *admitted == request_id.sequence && *offered == space_id => {
                    rooms.iter().map(|room| room.room_id.clone()).collect()
                }
                _ => Vec::new(),
            };
            koushi_protocol::command::RoomCommand::JoinSuggestedSpaceRooms {
                request_id,
                space_id,
                room_ids,
            }
        }
        other => other,
    }
}

pub(crate) fn space_member_forward_failure_action(
    command: &koushi_protocol::command::RoomCommand,
) -> Option<(RequestId, AppAction)> {
    match command {
        koushi_protocol::command::RoomCommand::LoadSpaceMembers {
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
        koushi_protocol::command::RoomCommand::LoadSpaceChildren {
            request_id,
            space_id,
            generation,
        } => Some((
            *request_id,
            AppAction::SpaceChildrenLoadFailed {
                space_id: space_id.clone(),
                generation: *generation,
                failure: OperationFailureKind::Sdk,
            },
        )),
        koushi_protocol::command::RoomCommand::InviteUserToSpace {
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
        koushi_protocol::command::RoomCommand::CancelSpaceInvite {
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
        koushi_protocol::command::RoomCommand::UpdateSpaceMemberRole {
            request_id,
            space_id,
            user_id,
            generation,
            ..
        } => Some((
            *request_id,
            AppAction::SpaceMemberRoleUpdateSettled {
                request_id: request_id.sequence,
                space_id: space_id.clone(),
                user_id: user_id.clone(),
                generation: *generation,
                outcome: koushi_state::SpaceMemberRoleUpdateOutcome::Failed(
                    koushi_state::SpaceMemberRoleFailureKind::Sdk,
                ),
                sent_revision: None,
                projection: None,
            },
        )),
        koushi_protocol::command::RoomCommand::SetSpaceChildSuggested {
            request_id,
            space_id,
            child_room_id,
            suggested,
        } => Some((
            *request_id,
            AppAction::SpaceChildSuggestedUpdateSettled {
                request_id: request_id.sequence,
                space_id: space_id.clone(),
                child_room_id: child_room_id.clone(),
                suggested: *suggested,
                failure: Some(OperationFailureKind::Sdk),
            },
        )),
        koushi_protocol::command::RoomCommand::JoinSuggestedSpaceRooms {
            request_id,
            space_id,
            ..
        } => Some((
            *request_id,
            AppAction::SuggestedRoomsJoinSettled {
                request_id: request_id.sequence,
                space_id: space_id.clone(),
                outcomes: Vec::new(),
            },
        )),
        _ => None,
    }
}

pub(crate) fn native_artifact_for_command(
    command: &CoreCommand,
) -> Option<(RequestId, NativeArtifactKind)> {
    match command {
        CoreCommand::Account(command) => native_artifact_for_account_command(command),
        _ => None,
    }
}

pub(crate) fn native_artifact_for_account_command(
    command: &AccountCommand,
) -> Option<(RequestId, NativeArtifactKind)> {
    match command {
        AccountCommand::ExportRoomKeys { request_id, .. } => {
            Some((*request_id, NativeArtifactKind::RoomKeyExportDestination))
        }
        AccountCommand::ImportRoomKeys { request_id, .. } => {
            Some((*request_id, NativeArtifactKind::RoomKeyImportSource))
        }
        AccountCommand::ExportHistory { request_id, .. } => {
            Some((*request_id, NativeArtifactKind::HistoryExportDirectory))
        }
        // Secure Backup setup, passphrase change (#927), and the identity
        // bootstrap (#1049) reveal the key on screen; only the optional save
        // command consumes a destination.
        AccountCommand::SaveSecureBackupRecoveryKey { request_id, .. } => {
            Some((*request_id, NativeArtifactKind::RecoveryKeyDestination))
        }
        _ => None,
    }
}

/// Core-owned admission policy over transport-neutral protocol commands.
pub trait CoreCommandPolicy {
    fn composer_draft_scope(&self) -> Option<ComposerDraftScope>;
    fn requires_ready_session(&self) -> bool;
}

impl CoreCommandPolicy for CoreCommand {
    fn composer_draft_scope(&self) -> Option<ComposerDraftScope> {
        match self {
            Self::App(AppCommand::SetComposerDraft {
                expected_account,
                room_id,
                ..
            }) => Some(ComposerDraftScope {
                account: expected_account.clone(),
                target: koushi_state::ComposerTarget::Main {
                    room_id: room_id.clone(),
                },
            }),
            Self::App(AppCommand::SetThreadComposerDraft {
                expected_account,
                room_id,
                root_event_id,
                ..
            }) => Some(ComposerDraftScope {
                account: expected_account.clone(),
                target: koushi_state::ComposerTarget::Thread {
                    room_id: room_id.clone(),
                    root_event_id: root_event_id.clone(),
                },
            }),
            Self::App(AppCommand::AcceptComposerDraft {
                expected_account,
                target,
                ..
            }) => Some(ComposerDraftScope {
                account: expected_account.clone(),
                target: target.clone(),
            }),
            Self::App(AppCommand::ScheduleSend {
                expected_account,
                room_id,
                thread_root_event_id,
                ..
            }) => Some(ComposerDraftScope {
                account: expected_account.clone(),
                target: thread_root_event_id
                    .as_ref()
                    .map(|root_event_id| koushi_state::ComposerTarget::Thread {
                        room_id: room_id.clone(),
                        root_event_id: root_event_id.clone(),
                    })
                    .unwrap_or_else(|| koushi_state::ComposerTarget::Main {
                        room_id: room_id.clone(),
                    }),
            }),
            Self::Timeline(
                TimelineCommand::SubmitText {
                    expected_account,
                    key,
                    ..
                }
                | TimelineCommand::SubmitReply {
                    expected_account,
                    key,
                    ..
                },
            ) => Some(ComposerDraftScope {
                account: expected_account.clone(),
                target: match &key.kind {
                    TimelineKind::Room { room_id } | TimelineKind::Focused { room_id, .. } => {
                        koushi_state::ComposerTarget::Main {
                            room_id: room_id.clone(),
                        }
                    }
                    TimelineKind::Thread {
                        room_id,
                        root_event_id,
                    } => koushi_state::ComposerTarget::Thread {
                        room_id: room_id.clone(),
                        root_event_id: root_event_id.clone(),
                    },
                },
            }),
            Self::App(_)
            | Self::Account(_)
            | Self::Sync(_)
            | Self::Room(_)
            | Self::Timeline(_)
            | Self::Search(_) => None,
        }
    }

    fn requires_ready_session(&self) -> bool {
        matches!(
            self,
            Self::Room(_) | Self::Timeline(_) | Self::Search(_) | Self::Sync(_)
        ) || matches!(self, Self::Account(command) if account_command_requires_ready_session(command))
            || matches!(
                self,
                Self::App(
                    AppCommand::OpenTimelineAtTimestamp { .. }
                        | AppCommand::RepairRoomTimeline { .. }
                        | AppCommand::EnterAnchoredTimeline { .. }
                        | AppCommand::ScheduleSend { .. }
                        | AppCommand::CancelScheduledSend { .. }
                        | AppCommand::RescheduleScheduledSend { .. }
                        | AppCommand::SetUploadStaging { .. }
                        | AppCommand::AcceptComposerDraft { .. }
                        | AppCommand::SeedComposerDraft { .. }
                        | AppCommand::DismissAssistantDraftMarker { .. }
                        | AppCommand::UpdateStagedUploadCaption { .. }
                        | AppCommand::UpdateStagedUploadCompression { .. }
                        | AppCommand::SelectStagedUploadOutput { .. }
                        | AppCommand::ClearUploadStaging { .. }
                        | AppCommand::RebuildSearchIndex { .. }
                        | AppCommand::SetRoomUrlPreviewOverride { .. }
                        | AppCommand::OpenFilesView { .. }
                        | AppCommand::OpenThreadsList { .. }
                        | AppCommand::CloseThreadsList { .. }
                        | AppCommand::PaginateThreadsList { .. }
                        | AppCommand::OpenScheduledSendsList { .. }
                        | AppCommand::CloseScheduledSendsList { .. }
                        | AppCommand::TimelineScrollAnchorUpdated { .. }
                )
            )
    }
}

fn account_command_requires_ready_session(command: &AccountCommand) -> bool {
    matches!(
        command,
        AccountCommand::RequestVerification { .. }
            | AccountCommand::RetryCurrentDeviceTrustDiscovery { .. }
            | AccountCommand::AcceptVerification { .. }
            | AccountCommand::ConfirmSasVerification { .. }
            | AccountCommand::CancelVerification { .. }
            | AccountCommand::BootstrapCrossSigning { .. }
            | AccountCommand::EnableKeyBackup { .. }
            | AccountCommand::ResetIdentity { .. }
            | AccountCommand::CancelIdentityReset { .. }
            | AccountCommand::SubmitIdentityResetAuth { .. }
            | AccountCommand::RefreshCurrentSessionStatus { .. }
            | AccountCommand::LoadAccountManagementCapabilities { .. }
            | AccountCommand::ChangePassword { .. }
            | AccountCommand::DeactivateAccount { .. }
            | AccountCommand::SubmitAccountManagementUia { .. }
            | AccountCommand::AccountNotifications { .. }
            | AccountCommand::ContactSecurity {
                request: koushi_protocol::command::ContactSecurityRequest::Load { .. }
                    | koushi_protocol::command::ContactSecurityRequest::RequestVerification { .. },
                ..
            }
            | AccountCommand::ExportRoomKeys { .. }
            | AccountCommand::ExportHistory { .. }
            | AccountCommand::StopHistoryExport { .. }
            | AccountCommand::RetryHistoryExport { .. }
            | AccountCommand::ImportRoomKeys { .. }
            | AccountCommand::BootstrapSecureBackup { .. }
            | AccountCommand::RecoverSecureBackup { .. }
            | AccountCommand::RetrySecureBackupInspection { .. }
            | AccountCommand::ChangeSecureBackupPassphrase { .. }
            | AccountCommand::SaveSecureBackupRecoveryKey { .. }
            | AccountCommand::ConfirmSecureBackupRecoveryKeySaved { .. }
            | AccountCommand::SetPresence { .. }
            | AccountCommand::SetDisplayName { .. }
            | AccountCommand::SetLocalUserAlias { .. }
            | AccountCommand::SetAvatar { .. }
            | AccountCommand::DownloadAvatarThumbnail { .. }
            | AccountCommand::CancelAvatarThumbnail { .. }
            | AccountCommand::IgnoreUser { .. }
            | AccountCommand::UnignoreUser { .. }
            | AccountCommand::ReportUser { .. }
            | AccountCommand::ProbeLocalEncryptionHealth { .. }
    )
}

pub(crate) fn timeline_composer_account_fence(
    command: &TimelineCommand,
) -> Option<(RequestId, &koushi_protocol::SessionKeyId)> {
    match command {
        TimelineCommand::SubmitText {
            request_id,
            expected_account,
            ..
        }
        | TimelineCommand::SubmitReply {
            request_id,
            expected_account,
            ..
        }
        | TimelineCommand::UploadAndSendMedia {
            request_id,
            expected_account,
            ..
        } => Some((*request_id, expected_account)),
        _ => None,
    }
}

pub(crate) fn search_scope_to_state(scope: &SearchScope) -> koushi_state::SearchScope {
    match scope {
        SearchScope::AllRooms => koushi_state::SearchScope::AllRooms,
        SearchScope::CurrentRoom { room_id } => koushi_state::SearchScope::CurrentRoom {
            room_id: room_id.clone(),
        },
        SearchScope::CurrentSpace { space_id } => koushi_state::SearchScope::CurrentSpace {
            space_id: space_id.clone(),
        },
    }
}

#[cfg(test)]
#[path = "command_policy/tests.rs"]
mod tests;
