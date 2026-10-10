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
/// Every other command passes through unchanged.
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
            // Closing User info only tears down; it must settle quietly after
            // sign-out, lock, or account switch unmounts the panel (#1024).
            CoreCommand::Account(AccountCommand::ContactSecurity {
                request_id: id,
                request: koushi_protocol::command::ContactSecurityRequest::Close,
            }),
        ] {
            assert!(!command.requires_ready_session());
        }

        for command in [
            CoreCommand::Sync(SyncCommand::Start { request_id: id }),
            CoreCommand::Account(AccountCommand::ContactSecurity {
                request_id: id,
                request: koushi_protocol::command::ContactSecurityRequest::Load {
                    user_id: "@contact:example.invalid".to_owned(),
                },
            }),
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
            CoreCommand::App(AppCommand::OpenScheduledSendsList {
                request_id: id,
                scope: koushi_state::ScheduledSendsScope::Home,
            }),
            CoreCommand::App(AppCommand::CloseScheduledSendsList { request_id: id }),
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

    fn synthetic_room(room_id: &str) -> koushi_state::RoomSummary {
        koushi_state::RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
            room_id: room_id.to_owned(),
            display_name: "Synthetic".to_owned(),
            display_label: "Synthetic".to_owned(),
            original_display_label: "Synthetic".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: koushi_state::RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            thread_unread_count: 0,
            thread_highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 1,
        }
    }

    #[test]
    fn leave_space_admission_keeps_only_joined_children_of_that_space() {
        let space_id = "!space:example.invalid";
        let state = AppState {
            spaces: vec![koushi_state::SpaceSummary {
                space_id: space_id.to_owned(),
                raw_name: None,
                display_name: "Synthetic Workspace".to_owned(),
                avatar: None,
                join_rule: None,
                child_room_ids: vec!["!child:example.invalid".to_owned()],
                parent_side_child_room_ids: vec!["!child:example.invalid".to_owned()],
            }],
            rooms: vec![
                synthetic_room("!child:example.invalid"),
                synthetic_room("!outside:example.invalid"),
            ],
            ..AppState::default()
        };
        let id = request(7);
        let admitted = admit_leave_space_command(
            &state,
            koushi_protocol::command::RoomCommand::LeaveSpace {
                request_id: id,
                space_id: space_id.to_owned(),
                child_room_ids: vec![
                    "!outside:example.invalid".to_owned(),
                    "!child:example.invalid".to_owned(),
                ],
            },
        );
        match admitted {
            koushi_protocol::command::RoomCommand::LeaveSpace {
                request_id,
                space_id: admitted_space_id,
                child_room_ids,
            } => {
                assert_eq!(request_id, id);
                assert_eq!(admitted_space_id, space_id);
                assert_eq!(child_room_ids, ["!child:example.invalid"]);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let passthrough = admit_leave_space_command(
            &state,
            koushi_protocol::command::RoomCommand::LeaveRoom {
                request_id: id,
                room_id: "!outside:example.invalid".to_owned(),
            },
        );
        assert!(matches!(
            passthrough,
            koushi_protocol::command::RoomCommand::LeaveRoom { room_id, .. }
                if room_id == "!outside:example.invalid"
        ));
    }
}
