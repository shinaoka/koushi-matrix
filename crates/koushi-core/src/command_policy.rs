use crate::composer_draft_lifecycle::ComposerDraftScope;
use crate::native_artifact::NativeArtifactKind;
use koushi_protocol::command::{
    AccountCommand, AppCommand, CoreCommand, SearchScope, TimelineCommand,
};
use koushi_protocol::ids::{RequestId, TimelineKind};
use koushi_state::{AppAction, OperationFailureKind};

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
        AccountCommand::BootstrapSecureBackup {
            request_id,
            request,
        }
        | AccountCommand::StartSessionBootstrap {
            request_id,
            request,
            ..
        } if request.recovery_key_destination_requested => {
            Some((*request_id, NativeArtifactKind::RecoveryKeyDestination))
        }
        AccountCommand::ChangeSecureBackupPassphrase {
            request_id,
            request,
        } if request.recovery_key_destination_requested => {
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
            | AccountCommand::ExportRoomKeys { .. }
            | AccountCommand::ImportRoomKeys { .. }
            | AccountCommand::BootstrapSecureBackup { .. }
            | AccountCommand::RecoverSecureBackup { .. }
            | AccountCommand::RetrySecureBackupInspection { .. }
            | AccountCommand::ChangeSecureBackupPassphrase { .. }
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
mod tests {
    use super::*;
    use koushi_protocol::{RuntimeConnectionId, SyncCommand};

    fn request(sequence: u64) -> RequestId {
        RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence,
        }
    }

    #[test]
    fn ready_admission_policy_stays_in_core() {
        let id = request(1);
        for command in [
            CoreCommand::Account(AccountCommand::SoftLogoutReauth {
                request_id: id,
                password: koushi_state::AuthSecret::new("synthetic"),
            }),
            CoreCommand::Account(AccountCommand::RetrySlidingSyncCapability { request_id: id }),
            CoreCommand::Account(AccountCommand::ResetLocalData { request_id: id }),
            CoreCommand::Account(AccountCommand::ChangeHomeserver { request_id: id }),
            CoreCommand::Account(AccountCommand::StartDeviceCleanup { request_id: id }),
            CoreCommand::Account(AccountCommand::EraseDeviceCleanupLocalDataAnyway {
                request_id: id,
            }),
        ] {
            assert!(!command.requires_ready_session());
        }

        for command in [
            CoreCommand::Sync(SyncCommand::Start { request_id: id }),
            CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
                request_id: id,
                room_id: "!room:example.invalid".to_owned(),
                timestamp_ms: 1,
            }),
            CoreCommand::App(AppCommand::ClearUploadStaging {
                request_id: id,
                target: koushi_state::ComposerTarget::Main {
                    room_id: "!room:example.invalid".to_owned(),
                },
            }),
        ] {
            assert!(command.requires_ready_session());
        }
    }

    #[test]
    fn composer_scope_and_timeline_account_fence_are_core_policy() {
        let expected_account = koushi_protocol::SessionKeyId {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        };
        let id = request(2);
        let command = CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: id,
            expected_account: expected_account.clone(),
            room_id: "!room:example.invalid".to_owned(),
            document: koushi_state::ComposerDocument::default(),
            revision: 1.into(),
        });
        assert!(command.composer_draft_scope().is_some());

        let timeline = TimelineCommand::SubmitText {
            request_id: id,
            expected_account: expected_account.clone(),
            submission_id: koushi_state::SubmissionId::new("submission"),
            key: koushi_protocol::TimelineKey::room(
                koushi_protocol::AccountKey("@user:example.invalid".to_owned()),
                "!room:example.invalid",
            ),
            transaction_id: "transaction".to_owned(),
            document: koushi_state::ComposerDocument::default(),
            draft_revision: 1.into(),
        };
        assert_eq!(
            timeline_composer_account_fence(&timeline),
            Some((id, &expected_account))
        );
    }
}
