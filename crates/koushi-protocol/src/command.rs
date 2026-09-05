//! Public command boundary. Every command carries a runtime-scoped
//! `RequestId`. Secret-bearing payloads redact `Debug`.

use crate::ids::{RequestId, RuntimeConnectionId};

#[derive(Debug)]
pub enum CoreCommand {
    App(AppCommand),
    Account(AccountCommand),
    Sync(SyncCommand),
    Room(RoomCommand),
    Timeline(TimelineCommand),
    Search(SearchCommand),
}

impl CoreCommand {
    /// The correlation id carried by every command.
    pub fn request_id(&self) -> RequestId {
        match self {
            Self::App(
                AppCommand::Shutdown { request_id }
                | AppCommand::SetComposerReplyTarget { request_id, .. }
                | AppCommand::CancelComposerReply { request_id }
                | AppCommand::SetComposerDraft { request_id, .. }
                | AppCommand::SetThreadComposerDraft { request_id, .. }
                | AppCommand::AcceptComposerDraft { request_id, .. }
                | AppCommand::SetUploadStaging { request_id, .. }
                | AppCommand::UpdateStagedUploadCaption { request_id, .. }
                | AppCommand::UpdateStagedUploadCompression { request_id, .. }
                | AppCommand::SelectStagedUploadOutput { request_id, .. }
                | AppCommand::ClearUploadStaging { request_id, .. }
                | AppCommand::ScheduleSend { request_id, .. }
                | AppCommand::CancelScheduledSend { request_id, .. }
                | AppCommand::RescheduleScheduledSend { request_id, .. }
                | AppCommand::OpenThread { request_id, .. }
                | AppCommand::CloseThread { request_id }
                | AppCommand::OpenFocusedContext { request_id, .. }
                | AppCommand::NavigateToEvent { request_id, .. }
                | AppCommand::OpenAnchoredTimeline { request_id, .. }
                | AppCommand::EnterAnchoredTimeline { request_id, .. }
                | AppCommand::OpenTimelineAtTimestamp { request_id, .. }
                | AppCommand::RepairRoomTimeline { request_id, .. }
                | AppCommand::TimelineScrollAnchorUpdated { request_id, .. }
                | AppCommand::CloseFocusedContext { request_id }
                | AppCommand::CloseSearch { request_id }
                | AppCommand::OpenInviteWorkflow { request_id, .. }
                | AppCommand::CloseInviteWorkflow { request_id }
                | AppCommand::SearchInviteTargets { request_id, .. }
                | AppCommand::SetInviteScope { request_id, .. }
                | AppCommand::SelectInviteTarget { request_id, .. }
                | AppCommand::RemoveInviteTarget { request_id, .. }
                | AppCommand::UpdateSettings { request_id, .. }
                | AppCommand::ImportLegacySettings { request_id, .. }
                | AppCommand::UpdateNavigationPreference { request_id, .. }
                | AppCommand::RebuildSearchIndex { request_id }
                | AppCommand::SetRoomUrlPreviewOverride { request_id, .. }
                | AppCommand::OpenActivity { request_id }
                | AppCommand::CloseActivity { request_id }
                | AppCommand::SetActivityTab { request_id, .. }
                | AppCommand::PaginateActivity { request_id, .. }
                | AppCommand::RetryActivityResolution { request_id }
                | AppCommand::MarkActivityRead { request_id, .. }
                | AppCommand::OpenFilesView { request_id, .. }
                | AppCommand::CloseFilesView { request_id }
                | AppCommand::OpenThreadsList { request_id, .. }
                | AppCommand::CloseThreadsList { request_id }
                | AppCommand::PaginateThreadsList { request_id, .. }
                | AppCommand::RecordLocalEncryptionHealth { request_id, .. }
                | AppCommand::UpdateNativeAttentionState { request_id, .. }
                | AppCommand::ObserveNativeWindowFocus { request_id, .. }
                | AppCommand::StartNativeAttentionDispatch { request_id, .. }
                | AppCommand::SettleNativeAttentionDispatch { request_id, .. }
                | AppCommand::UpdateJapaneseCatalogProfile { request_id, .. }
                | AppCommand::SelectRoomListFilter { request_id, .. },
            ) => *request_id,
            Self::Account(command) => match command {
                AccountCommand::LoginPassword { request_id, .. }
                | AccountCommand::DiscoverLogin { request_id, .. }
                | AccountCommand::StartOidcLogin { request_id, .. }
                | AccountCommand::CompleteOidcLogin { request_id, .. }
                | AccountCommand::RestoreSession { request_id, .. }
                | AccountCommand::RestoreLastSession { request_id }
                | AccountCommand::RetrySlidingSyncCapability { request_id }
                | AccountCommand::ChangeHomeserver { request_id }
                | AccountCommand::QuerySavedSessions { request_id }
                | AccountCommand::RefreshCurrentSessionStatus { request_id, .. }
                | AccountCommand::LoadAccountManagementCapabilities { request_id }
                | AccountCommand::ChangePassword { request_id, .. }
                | AccountCommand::DeactivateAccount { request_id, .. }
                | AccountCommand::SubmitAccountManagementUia { request_id, .. }
                | AccountCommand::SoftLogoutReauth { request_id, .. }
                | AccountCommand::ExportRoomKeys { request_id, .. }
                | AccountCommand::ImportRoomKeys { request_id, .. }
                | AccountCommand::BootstrapSecureBackup { request_id, .. }
                | AccountCommand::RecoverSecureBackup { request_id, .. }
                | AccountCommand::RetrySecureBackupInspection { request_id }
                | AccountCommand::ChangeSecureBackupPassphrase { request_id, .. }
                | AccountCommand::ProbeLocalEncryptionHealth { request_id }
                | AccountCommand::ResetLocalData { request_id }
                | AccountCommand::StartDeviceCleanup { request_id }
                | AccountCommand::SubmitDeviceCleanupUia { request_id, .. }
                | AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id }
                | AccountCommand::SubmitRecovery { request_id, .. }
                | AccountCommand::StartSessionBootstrap { request_id, .. }
                | AccountCommand::ConfirmSessionBootstrapSaved { request_id, .. }
                | AccountCommand::StartOwnUserSas { request_id, .. }
                | AccountCommand::RetryCurrentDeviceTrustDiscovery { request_id }
                | AccountCommand::RequestVerification { request_id, .. }
                | AccountCommand::AcceptVerification { request_id, .. }
                | AccountCommand::ConfirmSasVerification { request_id, .. }
                | AccountCommand::CancelVerification { request_id, .. }
                | AccountCommand::BootstrapCrossSigning { request_id, .. }
                | AccountCommand::EnableKeyBackup { request_id, .. }
                | AccountCommand::RestoreKeyBackup { request_id, .. }
                | AccountCommand::ResetIdentity { request_id }
                | AccountCommand::CancelIdentityReset { request_id, .. }
                | AccountCommand::SubmitIdentityResetAuth { request_id, .. }
                | AccountCommand::SetPresence { request_id, .. }
                | AccountCommand::SetDisplayName { request_id, .. }
                | AccountCommand::SetLocalUserAlias { request_id, .. }
                | AccountCommand::SetAvatar { request_id, .. }
                | AccountCommand::DownloadAvatarThumbnail { request_id, .. }
                | AccountCommand::IgnoreUser { request_id, .. }
                | AccountCommand::UnignoreUser { request_id, .. }
                | AccountCommand::ReportUser { request_id, .. }
                | AccountCommand::Logout { request_id }
                | AccountCommand::SwitchAccount { request_id, .. } => *request_id,
            },
            Self::Sync(command) => match command {
                SyncCommand::Start { request_id }
                | SyncCommand::Stop { request_id }
                | SyncCommand::Restart { request_id } => *request_id,
            },
            Self::Room(command) => match command {
                RoomCommand::CreateRoom { request_id, .. }
                | RoomCommand::CreatePublicDirectoryRoom { request_id, .. }
                | RoomCommand::CreateSpace { request_id, .. }
                | RoomCommand::SetSpaceChild { request_id, .. }
                | RoomCommand::InviteUser { request_id, .. }
                | RoomCommand::LoadSpaceMembers { request_id, .. }
                | RoomCommand::InviteUserToSpace { request_id, .. }
                | RoomCommand::CancelSpaceInvite { request_id, .. }
                | RoomCommand::InviteTargets { request_id, .. }
                | RoomCommand::AcceptInvite { request_id, .. }
                | RoomCommand::DeclineInvite { request_id, .. }
                | RoomCommand::StartDirectMessage { request_id, .. }
                | RoomCommand::JoinRoom { request_id, .. }
                | RoomCommand::LeaveRoom { request_id, .. }
                | RoomCommand::ForgetRoom { request_id, .. }
                | RoomCommand::SetTag { request_id, .. }
                | RoomCommand::RemoveTag { request_id, .. }
                | RoomCommand::PinEvent { request_id, .. }
                | RoomCommand::UnpinEvent { request_id, .. }
                | RoomCommand::RefreshPinnedEvents { request_id, .. }
                | RoomCommand::QueryDirectory { request_id, .. }
                | RoomCommand::PreviewJoinTarget { request_id, .. }
                | RoomCommand::DismissDirectoryPreview { request_id }
                | RoomCommand::JoinDirectoryRoom { request_id, .. }
                | RoomCommand::LoadRoomSettings { request_id, .. }
                | RoomCommand::QueryMentionCandidates { request_id, .. }
                | RoomCommand::UpdateRoomSetting { request_id, .. }
                | RoomCommand::ModerateRoomMember { request_id, .. }
                | RoomCommand::UpdateRoomMemberRole { request_id, .. }
                | RoomCommand::UpdateSpaceMemberRole { request_id, .. }
                | RoomCommand::SelectSpace { request_id, .. }
                | RoomCommand::ReorderSpaces { request_id, .. }
                | RoomCommand::SelectRoom { request_id, .. }
                | RoomCommand::MarkRoomAsRead { request_id, .. }
                | RoomCommand::MarkRoomAsUnread { request_id, .. }
                | RoomCommand::ForceRotateOutboundSession { request_id, .. }
                | RoomCommand::SetRoomNotificationMode { request_id, .. }
                | RoomCommand::ReportContent { request_id, .. }
                | RoomCommand::ReportRoom { request_id, .. } => *request_id,
            },
            Self::Timeline(command) => match command {
                TimelineCommand::Subscribe { request_id, .. }
                | TimelineCommand::EnsureSubscribed { request_id, .. }
                | TimelineCommand::ReplaySubscribed { request_id }
                | TimelineCommand::Unsubscribe { request_id, .. }
                | TimelineCommand::Paginate { request_id, .. }
                | TimelineCommand::CancelPagination { request_id, .. }
                | TimelineCommand::CancelLinkPreviews { request_id, .. }
                | TimelineCommand::RestoreTimelineAnchor { request_id, .. }
                | TimelineCommand::ObserveViewport { request_id, .. }
                | TimelineCommand::RepairGaps { request_id, .. }
                | TimelineCommand::SendText { request_id, .. }
                | TimelineCommand::SubmitText { request_id, .. }
                | TimelineCommand::SendReply { request_id, .. }
                | TimelineCommand::SubmitReply { request_id, .. }
                | TimelineCommand::ForwardMessage { request_id, .. }
                | TimelineCommand::LoadMessageSource { request_id, .. }
                | TimelineCommand::RequestRoomKey { request_id, .. }
                | TimelineCommand::RequestLateDecryption { request_id, .. }
                | TimelineCommand::RetrySend { request_id, .. }
                | TimelineCommand::CancelSend { request_id, .. }
                | TimelineCommand::UploadAndSendMedia { request_id, .. }
                | TimelineCommand::DownloadMedia { request_id, .. }
                | TimelineCommand::EditText { request_id, .. }
                | TimelineCommand::Redact { request_id, .. }
                | TimelineCommand::SendReaction { request_id, .. }
                | TimelineCommand::RedactReaction { request_id, .. }
                | TimelineCommand::SendReadReceipt { request_id, .. }
                | TimelineCommand::SetFullyRead { request_id, .. }
                | TimelineCommand::SetTyping { request_id, .. }
                | TimelineCommand::ToggleReaction { request_id, .. }
                | TimelineCommand::LoadLinkPreviews { request_id, .. }
                | TimelineCommand::HideLinkPreview { request_id, .. } => *request_id,
                TimelineCommand::BroadcastLinkPreviewPolicy { .. } => RequestId {
                    connection_id: RuntimeConnectionId(0),
                    sequence: 0,
                },
            },
            Self::Search(command) => match command {
                SearchCommand::Query { request_id, .. }
                | SearchCommand::Attachments { request_id, .. }
                | SearchCommand::StartHistoryCrawl { request_id, .. }
                | SearchCommand::StopHistoryCrawl { request_id, .. } => *request_id,
            },
        }
    }
}

#[derive(Debug)]
pub enum SyncCommand {
    Start { request_id: RequestId },
    Stop { request_id: RequestId },
    Restart { request_id: RequestId },
}

mod account;
mod app;
mod room;
mod search;
#[cfg(test)]
mod test_support;
mod timeline;

pub use account::{
    AccountCommand, RoomKeyExportRequest, RoomKeyImportRequest,
    SecureBackupPassphraseChangeRequest, SecureBackupSetupRequest, SetAvatarRequest,
};
pub use app::{AppCommand, EventNavigationMissingTargetPolicy};
pub use room::{CreateRoomOptions, CreateRoomParentSpace, CreateRoomVisibility, RoomCommand};
pub use search::{SearchCommand, SearchScope, ThreadsListCommand};
pub use timeline::{
    ImageUploadCompressionPolicy, ImageUploadCompressionState, ImageUploadDimensions,
    ImageUploadVariantInfo, ImageUploadVariantKind, InitialBackfillPolicy, KeyRequestOrigin,
    MediaDownloadSelection, TimelineCommand, UploadMediaKind, UploadMediaRequest,
    UploadMediaThumbnail,
};
