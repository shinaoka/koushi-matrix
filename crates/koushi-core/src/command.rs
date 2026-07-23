//! Public command boundary. Every command carries a runtime-scoped
//! `RequestId`. Secret-bearing payloads redact `Debug`.

use std::{fmt, path::PathBuf};

use koushi_state::{
    ActivityMarkReadTarget, ActivityTab, AttachmentFilter, AttachmentScope, AttachmentSort,
    DirectoryQuery, FilesViewScope, FormattedMessageDraft, IdentityResetAuthRequest,
    ImageUploadCompressionMode, InviteScopeSelection, JapaneseCatalogProfile,
    LocalEncryptionHealth, LoginRequest, MentionIntent, NativeAttentionDispatchId,
    NativeAttentionSoundOutcome, NativeAttentionState, PresenceKind, RecoveryRequest,
    RoomListFilter, RoomModerationAction, RoomSettingChange, RoomTagKind, SettingsPatch,
    StagedUploadCompressionChoice, StagedUploadItem, SubmissionId, TimelineScrollAnchor,
    VerificationCancelReason, VerificationTarget,
};
use serde::{Deserialize, Serialize};

use crate::event::TimelineViewportObservation;
use crate::ids::{
    AccountKey, RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey,
};

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
                | AppCommand::SelectStagedUploadVariant { request_id, .. }
                | AppCommand::ClearUploadStaging { request_id, .. }
                | AppCommand::ScheduleSend { request_id, .. }
                | AppCommand::CancelScheduledSend { request_id, .. }
                | AppCommand::RescheduleScheduledSend { request_id, .. }
                | AppCommand::OpenThread { request_id, .. }
                | AppCommand::CloseThread { request_id }
                | AppCommand::OpenFocusedContext { request_id, .. }
                | AppCommand::OpenAnchoredTimeline { request_id, .. }
                | AppCommand::AcknowledgeTimelineProjection { request_id, .. }
                | AppCommand::AcknowledgeTimelineBatchRendered { request_id, .. }
                | AppCommand::EnterAnchoredTimeline { request_id, .. }
                | AppCommand::OpenTimelineAtTimestamp { request_id, .. }
                | AppCommand::RepairRoomTimeline { request_id, .. }
                | AppCommand::TimelineScrollAnchorUpdated { request_id, .. }
                | AppCommand::CloseFocusedContext { request_id }
                | AppCommand::CloseSearch { request_id }
                | AppCommand::OpenInviteWorkflow { request_id, .. }
                | AppCommand::CloseInviteWorkflow { request_id }
                | AppCommand::SearchInviteTargets { request_id, .. }
                | AppCommand::SelectInviteTarget { request_id, .. }
                | AppCommand::RemoveInviteTarget { request_id, .. }
                | AppCommand::UpdateSettings { request_id, .. }
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
                | AppCommand::StartNativeAttentionDispatch { request_id, .. }
                | AppCommand::SettleNativeAttentionDispatch { request_id, .. }
                | AppCommand::UpdateJapaneseCatalogProfile { request_id, .. }
                | AppCommand::SelectRoomListFilter { request_id, .. },
            ) => *request_id,
            Self::Account(command) => match command {
                #[cfg(feature = "qa-bin")]
                AccountCommand::QaSetLocalDeviceBlacklisted { request_id, .. }
                | AccountCommand::QaRefreshDeviceKeysAndAssertKnown { request_id, .. } => {
                    *request_id
                }
                AccountCommand::LoginPassword { request_id, .. }
                | AccountCommand::DiscoverLogin { request_id, .. }
                | AccountCommand::StartOidcLogin { request_id, .. }
                | AccountCommand::CompleteOidcLogin { request_id, .. }
                | AccountCommand::RestoreSession { request_id, .. }
                | AccountCommand::RestoreLastSession { request_id }
                | AccountCommand::QuerySavedSessions { request_id }
                | AccountCommand::QueryDevices { request_id }
                | AccountCommand::LoadAccountManagementCapabilities { request_id }
                | AccountCommand::RenameDevice { request_id, .. }
                | AccountCommand::DeleteDevices { request_id, .. }
                | AccountCommand::ChangePassword { request_id, .. }
                | AccountCommand::DeactivateAccount { request_id, .. }
                | AccountCommand::SubmitAccountManagementUia { request_id, .. }
                | AccountCommand::SoftLogoutReauth { request_id, .. }
                | AccountCommand::ExportRoomKeys { request_id, .. }
                | AccountCommand::ImportRoomKeys { request_id, .. }
                | AccountCommand::BootstrapSecureBackup { request_id, .. }
                | AccountCommand::ChangeSecureBackupPassphrase { request_id, .. }
                | AccountCommand::ProbeLocalEncryptionHealth { request_id }
                | AccountCommand::ResetLocalData { request_id }
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
                | SyncCommand::Restart { request_id }
                | SyncCommand::SyncOnce { request_id } => *request_id,
            },
            Self::Room(command) => match command {
                RoomCommand::CreateRoom { request_id, .. }
                | RoomCommand::CreatePublicDirectoryRoom { request_id, .. }
                | RoomCommand::CreateSpace { request_id, .. }
                | RoomCommand::SetSpaceChild { request_id, .. }
                | RoomCommand::InviteUser { request_id, .. }
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
                | RoomCommand::QueryDirectory { request_id, .. }
                | RoomCommand::JoinDirectoryRoom { request_id, .. }
                | RoomCommand::LoadRoomSettings { request_id, .. }
                | RoomCommand::ReshareRoomKey { request_id, .. }
                | RoomCommand::UpdateRoomSetting { request_id, .. }
                | RoomCommand::ModerateRoomMember { request_id, .. }
                | RoomCommand::UpdateRoomMemberRole { request_id, .. }
                | RoomCommand::SelectSpace { request_id, .. }
                | RoomCommand::ReorderSpaces { request_id, .. }
                | RoomCommand::SelectRoom { request_id, .. }
                | RoomCommand::MarkRoomAsRead { request_id, .. }
                | RoomCommand::MarkRoomAsUnread { request_id, .. }
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

    /// Commands that require an exact `Ready` session before they are routed.
    /// External `SyncCommand`s are included; the restricted E2EE crypto lane
    /// used while gated is owned internally by `AccountActor`.
    pub fn requires_ready_session(&self) -> bool {
        matches!(
            self,
            Self::Room(_) | Self::Timeline(_) | Self::Search(_) | Self::Sync(_)
        ) || matches!(
            self,
            Self::Account(command) if command.requires_ready_session()
        ) || matches!(
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
                    | AppCommand::SelectStagedUploadVariant { .. }
                    | AppCommand::ClearUploadStaging { .. }
                    | AppCommand::RebuildSearchIndex { .. }
                    | AppCommand::SetRoomUrlPreviewOverride { .. }
                    | AppCommand::OpenFilesView { .. }
                    | AppCommand::OpenThreadsList { .. }
                    | AppCommand::CloseThreadsList { .. }
                    | AppCommand::PaginateThreadsList { .. }
                    | AppCommand::TimelineScrollAnchorUpdated { .. }
                    | AppCommand::AcknowledgeTimelineBatchRendered { .. }
            )
        )
    }
}

pub enum AppCommand {
    Shutdown {
        request_id: RequestId,
    },
    SetComposerReplyTarget {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    CancelComposerReply {
        request_id: RequestId,
    },
    SetComposerDraft {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        room_id: String,
        draft: String,
        revision: u64,
    },
    SetThreadComposerDraft {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        room_id: String,
        root_event_id: String,
        draft: String,
        revision: u64,
    },
    AcceptComposerDraft {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        target: koushi_state::ComposerTarget,
        submitted_revision: u64,
    },
    SetUploadStaging {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        items: Vec<StagedUploadItem>,
    },
    UpdateStagedUploadCaption {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        caption: Option<FormattedMessageDraft>,
    },
    UpdateStagedUploadCompression {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        compression_choice: StagedUploadCompressionChoice,
    },
    SelectStagedUploadVariant {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        variant_id: String,
    },
    ClearUploadStaging {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
    },
    ScheduleSend {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        send_at_ms: u64,
        draft_revision: u64,
    },
    CancelScheduledSend {
        request_id: RequestId,
        scheduled_id: String,
    },
    RescheduleScheduledSend {
        request_id: RequestId,
        scheduled_id: String,
        send_at_ms: u64,
    },
    OpenThread {
        request_id: RequestId,
        room_id: String,
        root_event_id: String,
    },
    CloseThread {
        request_id: RequestId,
    },
    OpenFocusedContext {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    /// Starts a main-pane Focused navigation whose anchor is withheld until
    /// the WebView acknowledges applying the matching actor projection.
    OpenAnchoredTimeline {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    /// Confirms that the canonical WebView timeline store applied one exact
    /// InitialItems projection. The projection request id remains stable when
    /// the active actor reprojects after a consumer remount.
    AcknowledgeTimelineProjection {
        request_id: RequestId,
        projection_request_id: RequestId,
        key: TimelineKey,
        generation: TimelineGeneration,
    },
    /// Confirms that the WebView committed a repair-produced timeline batch
    /// through layout. Every generation fence is required so a stale actor,
    /// timeline, repair, or batch cannot advance the repair scheduler.
    AcknowledgeTimelineBatchRendered {
        request_id: RequestId,
        key: TimelineKey,
        actor_generation: u64,
        timeline_generation: TimelineGeneration,
        repair_generation: u64,
        batch_id: TimelineBatchId,
    },
    EnterAnchoredTimeline {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    OpenTimelineAtTimestamp {
        request_id: RequestId,
        room_id: String,
        timestamp_ms: u64,
    },
    RepairRoomTimeline {
        request_id: RequestId,
        room_id: String,
    },
    TimelineScrollAnchorUpdated {
        request_id: RequestId,
        room_id: String,
        anchor: TimelineScrollAnchor,
    },
    CloseFocusedContext {
        request_id: RequestId,
    },
    CloseSearch {
        request_id: RequestId,
    },
    OpenInviteWorkflow {
        request_id: RequestId,
        room_id: String,
    },
    CloseInviteWorkflow {
        request_id: RequestId,
    },
    SearchInviteTargets {
        request_id: RequestId,
        room_id: String,
        query: String,
    },
    SelectInviteTarget {
        request_id: RequestId,
        room_id: String,
        user_id: String,
    },
    RemoveInviteTarget {
        request_id: RequestId,
        user_id: String,
    },
    UpdateSettings {
        request_id: RequestId,
        patch: SettingsPatch,
    },
    RebuildSearchIndex {
        request_id: RequestId,
    },
    SetRoomUrlPreviewOverride {
        request_id: RequestId,
        room_id: String,
        enabled: bool,
    },
    OpenActivity {
        request_id: RequestId,
    },
    CloseActivity {
        request_id: RequestId,
    },
    SetActivityTab {
        request_id: RequestId,
        tab: ActivityTab,
    },
    PaginateActivity {
        request_id: RequestId,
        tab: ActivityTab,
        cursor: Option<String>,
    },
    RetryActivityResolution {
        request_id: RequestId,
    },
    MarkActivityRead {
        request_id: RequestId,
        target: ActivityMarkReadTarget,
    },
    OpenFilesView {
        request_id: RequestId,
        scope: FilesViewScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    CloseFilesView {
        request_id: RequestId,
    },
    OpenThreadsList {
        request_id: RequestId,
        room_id: String,
    },
    CloseThreadsList {
        request_id: RequestId,
    },
    PaginateThreadsList {
        request_id: RequestId,
        room_id: String,
    },
    RecordLocalEncryptionHealth {
        request_id: RequestId,
        health: LocalEncryptionHealth,
    },
    UpdateNativeAttentionState {
        request_id: RequestId,
        attention: NativeAttentionState,
    },
    StartNativeAttentionDispatch {
        request_id: RequestId,
        dispatch_id: NativeAttentionDispatchId,
    },
    SettleNativeAttentionDispatch {
        request_id: RequestId,
        dispatch_id: NativeAttentionDispatchId,
        outcome: NativeAttentionSoundOutcome,
    },
    UpdateJapaneseCatalogProfile {
        request_id: RequestId,
        profile: JapaneseCatalogProfile,
    },
    SelectRoomListFilter {
        request_id: RequestId,
        filter: RoomListFilter,
    },
}

impl fmt::Debug for AppCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shutdown { request_id } => formatter
                .debug_struct("Shutdown")
                .field("request_id", request_id)
                .finish(),
            Self::SetComposerReplyTarget {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("SetComposerReplyTarget")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::CancelComposerReply { request_id } => formatter
                .debug_struct("CancelComposerReply")
                .field("request_id", request_id)
                .finish(),
            Self::SetComposerDraft {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("SetComposerDraft")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("draft", &"MessageBody(..)")
                .finish(),
            Self::SetThreadComposerDraft {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("SetThreadComposerDraft")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("root_event_id", &"EventId(..)")
                .field("draft", &"MessageBody(..)")
                .finish(),
            Self::AcceptComposerDraft { request_id, .. } => formatter
                .debug_struct("AcceptComposerDraft")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .finish(),
            Self::SetUploadStaging {
                request_id, items, ..
            } => formatter
                .debug_struct("SetUploadStaging")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .field("item_count", &items.len())
                .finish(),
            Self::UpdateStagedUploadCaption { request_id, .. } => formatter
                .debug_struct("UpdateStagedUploadCaption")
                .field("request_id", request_id)
                .field("staged_id", &"StagedUploadId(..)")
                .field("caption", &"MediaCaption(..)")
                .finish(),
            Self::UpdateStagedUploadCompression {
                request_id,
                compression_choice,
                ..
            } => formatter
                .debug_struct("UpdateStagedUploadCompression")
                .field("request_id", request_id)
                .field("staged_id", &"StagedUploadId(..)")
                .field("compression_choice", compression_choice)
                .finish(),
            Self::SelectStagedUploadVariant { request_id, .. } => formatter
                .debug_struct("SelectStagedUploadVariant")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .field("staged_id", &"StagedUploadId(..)")
                .field("variant_id", &"PreparedVariantId(..)")
                .finish(),
            Self::ClearUploadStaging { request_id, .. } => formatter
                .debug_struct("ClearUploadStaging")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .finish(),
            Self::ScheduleSend {
                request_id,
                send_at_ms,
                ..
            } => formatter
                .debug_struct("ScheduleSend")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("body", &"MessageBody(..)")
                .field("send_at_ms", &send_at_ms)
                .finish(),
            Self::CancelScheduledSend {
                request_id,
                scheduled_id,
            } => formatter
                .debug_struct("CancelScheduledSend")
                .field("request_id", request_id)
                .field("scheduled_id", scheduled_id)
                .finish(),
            Self::RescheduleScheduledSend {
                request_id,
                scheduled_id,
                send_at_ms,
            } => formatter
                .debug_struct("RescheduleScheduledSend")
                .field("request_id", request_id)
                .field("scheduled_id", scheduled_id)
                .field("send_at_ms", send_at_ms)
                .finish(),
            Self::OpenThread {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("OpenThread")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("root_event_id", &"EventId(..)")
                .finish(),
            Self::CloseThread { request_id } => formatter
                .debug_struct("CloseThread")
                .field("request_id", request_id)
                .finish(),
            Self::OpenFocusedContext {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("OpenFocusedContext")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::OpenAnchoredTimeline { request_id, .. } => formatter
                .debug_struct("OpenAnchoredTimeline")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::AcknowledgeTimelineProjection {
                request_id,
                projection_request_id,
                generation,
                ..
            } => formatter
                .debug_struct("AcknowledgeTimelineProjection")
                .field("request_id", request_id)
                .field("projection_request_id", projection_request_id)
                .field("key", &"TimelineKey(..)")
                .field("generation", generation)
                .finish(),
            Self::AcknowledgeTimelineBatchRendered {
                request_id,
                actor_generation,
                timeline_generation,
                repair_generation,
                batch_id,
                ..
            } => formatter
                .debug_struct("AcknowledgeTimelineBatchRendered")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("actor_generation", actor_generation)
                .field("timeline_generation", timeline_generation)
                .field("repair_generation", repair_generation)
                .field("batch_id", batch_id)
                .finish(),
            Self::EnterAnchoredTimeline {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("EnterAnchoredTimeline")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::OpenTimelineAtTimestamp { request_id, .. } => formatter
                .debug_struct("OpenTimelineAtTimestamp")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("timestamp_ms", &"Timestamp(..)")
                .finish(),
            Self::RepairRoomTimeline { request_id, .. } => formatter
                .debug_struct("RepairRoomTimeline")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::TimelineScrollAnchorUpdated {
                request_id, anchor, ..
            } => formatter
                .debug_struct("TimelineScrollAnchorUpdated")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .field("offset_px", &anchor.offset_px)
                .field("updated_at_ms", &anchor.updated_at_ms)
                .finish(),
            Self::CloseFocusedContext { request_id } => formatter
                .debug_struct("CloseFocusedContext")
                .field("request_id", request_id)
                .finish(),
            Self::CloseSearch { request_id } => formatter
                .debug_struct("CloseSearch")
                .field("request_id", request_id)
                .finish(),
            Self::OpenInviteWorkflow { request_id, .. } => formatter
                .debug_struct("OpenInviteWorkflow")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::CloseInviteWorkflow { request_id } => formatter
                .debug_struct("CloseInviteWorkflow")
                .field("request_id", request_id)
                .finish(),
            Self::SearchInviteTargets {
                request_id, query, ..
            } => formatter
                .debug_struct("SearchInviteTargets")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("query_len", &query.len())
                .finish(),
            Self::SelectInviteTarget { request_id, .. } => formatter
                .debug_struct("SelectInviteTarget")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::RemoveInviteTarget { request_id, .. } => formatter
                .debug_struct("RemoveInviteTarget")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::UpdateSettings { request_id, patch } => formatter
                .debug_struct("UpdateSettings")
                .field("request_id", request_id)
                .field("patch_fields", &settings_patch_field_names(patch))
                .finish(),
            Self::RebuildSearchIndex { request_id } => formatter
                .debug_struct("RebuildSearchIndex")
                .field("request_id", request_id)
                .finish(),
            Self::SetRoomUrlPreviewOverride {
                request_id,
                enabled,
                ..
            } => formatter
                .debug_struct("SetRoomUrlPreviewOverride")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("enabled", enabled)
                .finish(),
            Self::OpenActivity { request_id } => formatter
                .debug_struct("OpenActivity")
                .field("request_id", request_id)
                .finish(),
            Self::CloseActivity { request_id } => formatter
                .debug_struct("CloseActivity")
                .field("request_id", request_id)
                .finish(),
            Self::SetActivityTab { request_id, tab } => formatter
                .debug_struct("SetActivityTab")
                .field("request_id", request_id)
                .field("tab", tab)
                .finish(),
            Self::PaginateActivity {
                request_id,
                tab,
                cursor,
            } => formatter
                .debug_struct("PaginateActivity")
                .field("request_id", request_id)
                .field("tab", tab)
                .field("cursor", &cursor.as_ref().map(|_| "PageToken(..)"))
                .finish(),
            Self::RetryActivityResolution { request_id } => formatter
                .debug_struct("RetryActivityResolution")
                .field("request_id", request_id)
                .finish(),
            Self::MarkActivityRead { request_id, target } => formatter
                .debug_struct("MarkActivityRead")
                .field("request_id", request_id)
                .field("target", target)
                .finish(),
            Self::OpenFilesView {
                request_id,
                scope,
                filter,
                sort,
            } => formatter
                .debug_struct("OpenFilesView")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .finish(),
            Self::CloseFilesView { request_id } => formatter
                .debug_struct("CloseFilesView")
                .field("request_id", request_id)
                .finish(),
            Self::OpenThreadsList { request_id, .. } => formatter
                .debug_struct("OpenThreadsList")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::CloseThreadsList { request_id } => formatter
                .debug_struct("CloseThreadsList")
                .field("request_id", request_id)
                .finish(),
            Self::PaginateThreadsList { request_id, .. } => formatter
                .debug_struct("PaginateThreadsList")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RecordLocalEncryptionHealth { request_id, health } => formatter
                .debug_struct("RecordLocalEncryptionHealth")
                .field("request_id", request_id)
                .field("health", health)
                .finish(),
            Self::UpdateNativeAttentionState {
                request_id,
                attention,
            } => formatter
                .debug_struct("UpdateNativeAttentionState")
                .field("request_id", request_id)
                .field("unread_count", &attention.summary.unread_count)
                .field("highlight_count", &attention.summary.highlight_count)
                .field("badge_count", &attention.summary.badge_count)
                .field("dispatch", &attention.dispatch.kind())
                .field(
                    "candidate",
                    &attention
                        .summary
                        .candidate
                        .as_ref()
                        .map(|_| "AttentionCandidate(..)"),
                )
                .finish(),
            Self::StartNativeAttentionDispatch {
                request_id,
                dispatch_id,
            } => formatter
                .debug_struct("StartNativeAttentionDispatch")
                .field("request_id", request_id)
                .field("dispatch_id", dispatch_id)
                .finish(),
            Self::SettleNativeAttentionDispatch {
                request_id,
                dispatch_id,
                outcome,
            } => formatter
                .debug_struct("SettleNativeAttentionDispatch")
                .field("request_id", request_id)
                .field("dispatch_id", dispatch_id)
                .field("outcome", outcome)
                .finish(),
            Self::UpdateJapaneseCatalogProfile {
                request_id,
                profile,
            } => formatter
                .debug_struct("UpdateJapaneseCatalogProfile")
                .field("request_id", request_id)
                .field("catalog_locale", &profile.catalog_locale)
                .field("complete", &profile.complete)
                .field("missing_count", &profile.missing_message_ids.len())
                .finish(),
            Self::SelectRoomListFilter { request_id, filter } => formatter
                .debug_struct("SelectRoomListFilter")
                .field("request_id", request_id)
                .field("filter", filter)
                .finish(),
        }
    }
}

fn settings_patch_field_names(patch: &SettingsPatch) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if patch.locale.is_some() {
        fields.push("locale");
    }
    if patch.appearance.is_some() {
        fields.push("appearance");
    }
    if patch.typography.is_some() {
        fields.push("typography");
    }
    if patch.keyboard.is_some() {
        fields.push("keyboard");
    }
    if patch.notifications.is_some() {
        fields.push("notifications");
    }
    if patch.display.is_some() {
        fields.push("display");
    }
    fields
}

#[derive(Clone, Eq, PartialEq)]
pub struct RoomKeyExportRequest {
    pub destination_path: PathBuf,
    pub passphrase: koushi_state::AuthSecret,
}

impl fmt::Debug for RoomKeyExportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomKeyExportRequest")
            .field("destination_path", &"DestinationPath(..)")
            .field("passphrase", &"AuthSecret(..)")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct RoomKeyImportRequest {
    pub source_path: PathBuf,
    pub passphrase: koushi_state::AuthSecret,
}

impl fmt::Debug for RoomKeyImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomKeyImportRequest")
            .field("source_path", &"SourcePath(..)")
            .field("passphrase", &"AuthSecret(..)")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SecureBackupSetupRequest {
    pub passphrase: Option<koushi_state::AuthSecret>,
    pub recovery_key_destination_path: Option<PathBuf>,
}

impl fmt::Debug for SecureBackupSetupRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureBackupSetupRequest")
            .field("has_passphrase", &self.passphrase.is_some())
            .field(
                "has_recovery_key_destination_path",
                &self.recovery_key_destination_path.is_some(),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SecureBackupPassphraseChangeRequest {
    pub old_secret: koushi_state::AuthSecret,
    pub new_passphrase: koushi_state::AuthSecret,
    pub recovery_key_destination_path: Option<PathBuf>,
}

impl fmt::Debug for SecureBackupPassphraseChangeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureBackupPassphraseChangeRequest")
            .field(
                "has_recovery_key_destination_path",
                &self.recovery_key_destination_path.is_some(),
            )
            .field("old_secret", &"AuthSecret(..)")
            .field("new_passphrase", &"AuthSecret(..)")
            .finish()
    }
}

// LoginRequest and RecoveryRequest redact their own Debug in
// koushi-state (username, password, device name, recovery secret).
pub enum AccountCommand {
    DiscoverLogin {
        request_id: RequestId,
        homeserver: String,
    },
    StartOidcLogin {
        request_id: RequestId,
        homeserver: String,
    },
    CompleteOidcLogin {
        request_id: RequestId,
        callback_url: String,
    },
    LoginPassword {
        request_id: RequestId,
        request: LoginRequest,
    },
    RestoreSession {
        request_id: RequestId,
        account_key: AccountKey,
    },
    /// Restore whichever account the last-session pointer designates. The
    /// pointer is resolved inside the StoreActor/AccountActor — the transport
    /// adapter never reads the credential store. A missing pointer (or
    /// missing session data) is a NORMAL outcome reported as
    /// `CoreFailure::SessionNotFound`; the UI goes to login quietly.
    RestoreLastSession {
        request_id: RequestId,
    },
    /// List saved sessions (homeserver / user_id / device_id only — never
    /// secrets). Answered by `AccountEvent::SavedSessionsListed`.
    QuerySavedSessions {
        request_id: RequestId,
    },
    QueryDevices {
        request_id: RequestId,
    },
    LoadAccountManagementCapabilities {
        request_id: RequestId,
    },
    RenameDevice {
        request_id: RequestId,
        device_ordinal: u64,
        display_name: String,
    },
    DeleteDevices {
        request_id: RequestId,
        device_ordinals: Vec<u64>,
        auth: Option<IdentityResetAuthRequest>,
    },
    ChangePassword {
        request_id: RequestId,
        new_password: koushi_state::AuthSecret,
    },
    DeactivateAccount {
        request_id: RequestId,
        erase_data: bool,
    },
    SubmitAccountManagementUia {
        request_id: RequestId,
        flow_id: u64,
        auth: IdentityResetAuthRequest,
    },
    SoftLogoutReauth {
        request_id: RequestId,
        password: koushi_state::AuthSecret,
    },
    ExportRoomKeys {
        request_id: RequestId,
        request: RoomKeyExportRequest,
    },
    ImportRoomKeys {
        request_id: RequestId,
        request: RoomKeyImportRequest,
    },
    BootstrapSecureBackup {
        request_id: RequestId,
        request: SecureBackupSetupRequest,
    },
    ChangeSecureBackupPassphrase {
        request_id: RequestId,
        request: SecureBackupPassphraseChangeRequest,
    },
    ProbeLocalEncryptionHealth {
        request_id: RequestId,
    },
    ResetLocalData {
        request_id: RequestId,
    },
    SubmitRecovery {
        request_id: RequestId,
        request: RecoveryRequest,
    },
    StartSessionBootstrap {
        request_id: RequestId,
        flow_id: u64,
        auth: Option<koushi_state::AuthSecret>,
        request: SecureBackupSetupRequest,
    },
    ConfirmSessionBootstrapSaved {
        request_id: RequestId,
        flow_id: u64,
    },
    StartOwnUserSas {
        request_id: RequestId,
        flow_id: u64,
    },
    RetryCurrentDeviceTrustDiscovery {
        request_id: RequestId,
    },
    RequestVerification {
        request_id: RequestId,
        target: VerificationTarget,
    },
    AcceptVerification {
        request_id: RequestId,
        flow_id: u64,
    },
    ConfirmSasVerification {
        request_id: RequestId,
        flow_id: u64,
    },
    CancelVerification {
        request_id: RequestId,
        flow_id: u64,
        reason: VerificationCancelReason,
    },
    BootstrapCrossSigning {
        request_id: RequestId,
        auth: Option<koushi_state::AuthSecret>,
    },
    EnableKeyBackup {
        request_id: RequestId,
        passphrase: Option<koushi_state::AuthSecret>,
    },
    RestoreKeyBackup {
        request_id: RequestId,
        version: Option<String>,
        request: RecoveryRequest,
    },
    #[cfg(feature = "qa-bin")]
    QaSetLocalDeviceBlacklisted {
        request_id: RequestId,
        target: VerificationTarget,
        room_id: String,
        acknowledged: tokio::sync::oneshot::Sender<Result<(), ()>>,
    },
    #[cfg(feature = "qa-bin")]
    QaRefreshDeviceKeysAndAssertKnown {
        request_id: RequestId,
        target: VerificationTarget,
        acknowledged: tokio::sync::oneshot::Sender<Result<(), ()>>,
    },
    ResetIdentity {
        request_id: RequestId,
    },
    CancelIdentityReset {
        request_id: RequestId,
        flow_id: u64,
    },
    SubmitIdentityResetAuth {
        request_id: RequestId,
        flow_id: u64,
        request: IdentityResetAuthRequest,
    },
    SetPresence {
        request_id: RequestId,
        presence: PresenceKind,
    },
    SetDisplayName {
        request_id: RequestId,
        display_name: Option<String>,
    },
    SetLocalUserAlias {
        request_id: RequestId,
        user_id: String,
        alias: Option<String>,
    },
    SetAvatar {
        request_id: RequestId,
        request: SetAvatarRequest,
    },
    DownloadAvatarThumbnail {
        request_id: RequestId,
        mxc_uri: String,
    },
    IgnoreUser {
        request_id: RequestId,
        user_id: String,
    },
    UnignoreUser {
        request_id: RequestId,
        user_id: String,
    },
    ReportUser {
        request_id: RequestId,
        user_id: String,
        reason: String,
    },
    Logout {
        request_id: RequestId,
    },
    SwitchAccount {
        request_id: RequestId,
        account_key: AccountKey,
    },
}

impl AccountCommand {
    pub fn requires_ready_session(&self) -> bool {
        #[cfg(feature = "qa-bin")]
        if matches!(self, Self::QaRefreshDeviceKeysAndAssertKnown { .. }) {
            return true;
        }

        matches!(
            self,
            Self::RequestVerification { .. }
                | Self::RetryCurrentDeviceTrustDiscovery { .. }
                | Self::AcceptVerification { .. }
                | Self::ConfirmSasVerification { .. }
                | Self::CancelVerification { .. }
                | Self::BootstrapCrossSigning { .. }
                | Self::EnableKeyBackup { .. }
                | Self::ResetIdentity { .. }
                | Self::CancelIdentityReset { .. }
                | Self::SubmitIdentityResetAuth { .. }
                | Self::QueryDevices { .. }
                | Self::LoadAccountManagementCapabilities { .. }
                | Self::RenameDevice { .. }
                | Self::DeleteDevices { .. }
                | Self::ChangePassword { .. }
                | Self::DeactivateAccount { .. }
                | Self::SubmitAccountManagementUia { .. }
                | Self::ExportRoomKeys { .. }
                | Self::ImportRoomKeys { .. }
                | Self::BootstrapSecureBackup { .. }
                | Self::ChangeSecureBackupPassphrase { .. }
                | Self::SetPresence { .. }
                | Self::SetDisplayName { .. }
                | Self::SetLocalUserAlias { .. }
                | Self::SetAvatar { .. }
                | Self::DownloadAvatarThumbnail { .. }
                | Self::IgnoreUser { .. }
                | Self::UnignoreUser { .. }
                | Self::ReportUser { .. }
                | Self::ProbeLocalEncryptionHealth { .. }
                | Self::ResetLocalData { .. }
        )
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SetAvatarRequest {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for SetAvatarRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetAvatarRequest")
            .field("mime_type", &self.mime_type)
            .field("bytes", &"AvatarBytes(..)")
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}

impl fmt::Debug for AccountCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiscoverLogin { request_id, .. } => formatter
                .debug_struct("DiscoverLogin")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .finish(),
            Self::StartOidcLogin { request_id, .. } => formatter
                .debug_struct("StartOidcLogin")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .finish(),
            Self::CompleteOidcLogin { request_id, .. } => formatter
                .debug_struct("CompleteOidcLogin")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .field("callback_url", &"CallbackUrl(..)")
                .finish(),
            Self::LoginPassword {
                request_id,
                request,
            } => formatter
                .debug_struct("LoginPassword")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::RestoreSession {
                request_id,
                account_key,
            } => formatter
                .debug_struct("RestoreSession")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::RestoreLastSession { request_id } => formatter
                .debug_struct("RestoreLastSession")
                .field("request_id", request_id)
                .finish(),
            Self::QuerySavedSessions { request_id } => formatter
                .debug_struct("QuerySavedSessions")
                .field("request_id", request_id)
                .finish(),
            Self::QueryDevices { request_id } => formatter
                .debug_struct("QueryDevices")
                .field("request_id", request_id)
                .finish(),
            Self::LoadAccountManagementCapabilities { request_id } => formatter
                .debug_struct("LoadAccountManagementCapabilities")
                .field("request_id", request_id)
                .finish(),
            Self::RenameDevice {
                request_id,
                device_ordinal,
                ..
            } => formatter
                .debug_struct("RenameDevice")
                .field("request_id", request_id)
                .field("device_ordinal", device_ordinal)
                .field("display_name", &"DeviceDisplayName(..)")
                .finish(),
            Self::DeleteDevices {
                request_id,
                device_ordinals,
                auth,
            } => formatter
                .debug_struct("DeleteDevices")
                .field("request_id", request_id)
                .field("device_ordinals", device_ordinals)
                .field("auth", auth)
                .finish(),
            Self::ChangePassword { request_id, .. } => formatter
                .debug_struct("ChangePassword")
                .field("request_id", request_id)
                .field("new_password", &"AuthSecret(..)")
                .finish(),
            Self::DeactivateAccount {
                request_id,
                erase_data,
            } => formatter
                .debug_struct("DeactivateAccount")
                .field("request_id", request_id)
                .field("erase_data", erase_data)
                .finish(),
            Self::SubmitAccountManagementUia {
                request_id,
                flow_id,
                auth,
            } => formatter
                .debug_struct("SubmitAccountManagementUia")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("auth", auth)
                .finish(),
            Self::SoftLogoutReauth { request_id, .. } => formatter
                .debug_struct("SoftLogoutReauth")
                .field("request_id", request_id)
                .field("password", &"AuthSecret(..)")
                .finish(),
            Self::ExportRoomKeys {
                request_id,
                request,
            } => formatter
                .debug_struct("ExportRoomKeys")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::ImportRoomKeys {
                request_id,
                request,
            } => formatter
                .debug_struct("ImportRoomKeys")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::BootstrapSecureBackup {
                request_id,
                request,
            } => formatter
                .debug_struct("BootstrapSecureBackup")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::ChangeSecureBackupPassphrase {
                request_id,
                request,
            } => formatter
                .debug_struct("ChangeSecureBackupPassphrase")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::ProbeLocalEncryptionHealth { request_id } => formatter
                .debug_struct("ProbeLocalEncryptionHealth")
                .field("request_id", request_id)
                .finish(),
            Self::ResetLocalData { request_id } => formatter
                .debug_struct("ResetLocalData")
                .field("request_id", request_id)
                .finish(),
            Self::SubmitRecovery {
                request_id,
                request,
            } => formatter
                .debug_struct("SubmitRecovery")
                .field("request_id", request_id)
                .field("request", request)
                .finish(),
            Self::StartSessionBootstrap {
                request_id,
                flow_id,
                auth,
                request,
            } => formatter
                .debug_struct("StartSessionBootstrap")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("has_auth", &auth.is_some())
                .field("request", request)
                .finish(),
            Self::ConfirmSessionBootstrapSaved {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("ConfirmSessionBootstrapSaved")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::StartOwnUserSas {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("StartOwnUserSas")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::RetryCurrentDeviceTrustDiscovery { request_id } => formatter
                .debug_struct("RetryCurrentDeviceTrustDiscovery")
                .field("request_id", request_id)
                .finish(),
            Self::RequestVerification { request_id, .. } => formatter
                .debug_struct("RequestVerification")
                .field("request_id", request_id)
                .field("target", &"VerificationTarget(..)")
                .finish(),
            Self::AcceptVerification {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("AcceptVerification")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::ConfirmSasVerification {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("ConfirmSasVerification")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::CancelVerification {
                request_id,
                flow_id,
                reason,
            } => formatter
                .debug_struct("CancelVerification")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("reason", reason)
                .finish(),
            Self::BootstrapCrossSigning { request_id, auth } => formatter
                .debug_struct("BootstrapCrossSigning")
                .field("request_id", request_id)
                .field("auth", auth)
                .finish(),
            Self::EnableKeyBackup {
                request_id,
                passphrase,
            } => formatter
                .debug_struct("EnableKeyBackup")
                .field("request_id", request_id)
                .field("passphrase", passphrase)
                .finish(),
            Self::RestoreKeyBackup {
                request_id,
                version,
                request,
            } => formatter
                .debug_struct("RestoreKeyBackup")
                .field("request_id", request_id)
                .field("version", &version.as_ref().map(|_| "BackupVersion(..)"))
                .field("request", request)
                .finish(),
            #[cfg(feature = "qa-bin")]
            Self::QaSetLocalDeviceBlacklisted { request_id, .. } => formatter
                .debug_struct("QaSetLocalDeviceBlacklisted")
                .field("request_id", request_id)
                .field("target", &"<redacted>")
                .finish(),
            #[cfg(feature = "qa-bin")]
            Self::QaRefreshDeviceKeysAndAssertKnown { request_id, .. } => formatter
                .debug_struct("QaRefreshDeviceKeysAndAssertKnown")
                .field("request_id", request_id)
                .field("target", &"<redacted>")
                .finish(),
            Self::ResetIdentity { request_id } => formatter
                .debug_struct("ResetIdentity")
                .field("request_id", request_id)
                .finish(),
            Self::CancelIdentityReset {
                request_id,
                flow_id,
            } => formatter
                .debug_struct("CancelIdentityReset")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .finish(),
            Self::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request,
            } => formatter
                .debug_struct("SubmitIdentityResetAuth")
                .field("request_id", request_id)
                .field("flow_id", flow_id)
                .field("request", request)
                .finish(),
            Self::SetPresence {
                request_id,
                presence,
            } => formatter
                .debug_struct("SetPresence")
                .field("request_id", request_id)
                .field("presence", presence)
                .finish(),
            Self::SetDisplayName { request_id, .. } => formatter
                .debug_struct("SetDisplayName")
                .field("request_id", request_id)
                .field("display_name", &"ProfileDisplayName(..)")
                .finish(),
            Self::SetLocalUserAlias { request_id, .. } => formatter
                .debug_struct("SetLocalUserAlias")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .field("alias", &"LocalUserAlias(..)")
                .finish(),
            Self::SetAvatar {
                request_id,
                request,
            } => formatter
                .debug_struct("SetAvatar")
                .field("request_id", request_id)
                .field("mime_type", &request.mime_type)
                .field("bytes", &"AvatarBytes(..)")
                .field("bytes_len", &request.bytes.len())
                .finish(),
            Self::DownloadAvatarThumbnail { request_id, .. } => formatter
                .debug_struct("DownloadAvatarThumbnail")
                .field("request_id", request_id)
                .field("mxc_uri", &"MxcUri(..)")
                .finish(),
            Self::IgnoreUser { request_id, .. } => formatter
                .debug_struct("IgnoreUser")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::UnignoreUser { request_id, .. } => formatter
                .debug_struct("UnignoreUser")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::ReportUser { request_id, .. } => formatter
                .debug_struct("ReportUser")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .field("reason", &"ReportReason(..)")
                .finish(),
            Self::Logout { request_id } => formatter
                .debug_struct("Logout")
                .field("request_id", request_id)
                .finish(),
            Self::SwitchAccount {
                request_id,
                account_key,
            } => formatter
                .debug_struct("SwitchAccount")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
        }
    }
}

#[derive(Debug)]
pub enum SyncCommand {
    Start { request_id: RequestId },
    Stop { request_id: RequestId },
    Restart { request_id: RequestId },
    SyncOnce { request_id: RequestId },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomOptions {
    pub name: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub alias_localpart: Option<String>,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default)]
    pub visibility: CreateRoomVisibility,
    #[serde(default)]
    pub parent_space: Option<CreateRoomParentSpace>,
}

impl fmt::Debug for CreateRoomOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateRoomOptions")
            .field("name", &"RoomName(..)")
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field(
                "alias_localpart",
                &self
                    .alias_localpart
                    .as_ref()
                    .map(|_| "RoomAliasLocalpart(..)"),
            )
            .field("encrypted", &self.encrypted)
            .field("visibility", &self.visibility)
            .field("parent_space", &self.parent_space)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreateRoomVisibility {
    #[default]
    Private,
    Public,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomParentSpace {
    pub space_id: String,
    pub via_server: String,
}

impl fmt::Debug for CreateRoomParentSpace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateRoomParentSpace")
            .field("space_id", &"RoomId(..)")
            .field("via_server", &"ServerName(..)")
            .finish()
    }
}

pub enum RoomCommand {
    CreateRoom {
        request_id: RequestId,
        options: CreateRoomOptions,
    },
    CreatePublicDirectoryRoom {
        request_id: RequestId,
        name: String,
        alias_localpart: String,
    },
    CreateSpace {
        request_id: RequestId,
        name: String,
    },
    SetSpaceChild {
        request_id: RequestId,
        space_id: String,
        child_room_id: String,
        via_server: String,
    },
    InviteUser {
        request_id: RequestId,
        room_id: String,
        user_id: String,
    },
    InviteTargets {
        request_id: RequestId,
        room_id: String,
        user_ids: Vec<String>,
        scope: InviteScopeSelection,
    },
    AcceptInvite {
        request_id: RequestId,
        room_id: String,
    },
    DeclineInvite {
        request_id: RequestId,
        room_id: String,
    },
    StartDirectMessage {
        request_id: RequestId,
        user_id: String,
    },
    JoinRoom {
        request_id: RequestId,
        room_id: String,
    },
    LeaveRoom {
        request_id: RequestId,
        room_id: String,
    },
    ForgetRoom {
        request_id: RequestId,
        room_id: String,
    },
    SetTag {
        request_id: RequestId,
        room_id: String,
        tag: RoomTagKind,
        order: Option<f64>,
    },
    RemoveTag {
        request_id: RequestId,
        room_id: String,
        tag: RoomTagKind,
    },
    PinEvent {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    UnpinEvent {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    QueryDirectory {
        request_id: RequestId,
        query: DirectoryQuery,
    },
    JoinDirectoryRoom {
        request_id: RequestId,
        alias: String,
        via_server: Option<String>,
    },
    LoadRoomSettings {
        request_id: RequestId,
        room_id: String,
    },
    ReshareRoomKey {
        request_id: RequestId,
        room_id: String,
    },
    UpdateRoomSetting {
        request_id: RequestId,
        room_id: String,
        change: RoomSettingChange,
    },
    ModerateRoomMember {
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
        reason: Option<String>,
    },
    UpdateRoomMemberRole {
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        power_level: i64,
    },
    SelectSpace {
        request_id: RequestId,
        space_id: Option<String>,
    },
    ReorderSpaces {
        request_id: RequestId,
        space_ids: Vec<String>,
    },
    /// User-intent lane: room selection is request-id correlated and must be
    /// routed through the reliable command path, not a drop-on-full background
    /// queue.
    SelectRoom {
        request_id: RequestId,
        room_id: String,
    },
    MarkRoomAsRead {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    MarkRoomAsUnread {
        request_id: RequestId,
        room_id: String,
        unread: bool,
    },
    SetRoomNotificationMode {
        request_id: RequestId,
        room_id: String,
        mode: koushi_state::RoomNotificationMode,
    },
    ReportContent {
        request_id: RequestId,
        room_id: String,
        event_id: String,
        reason: Option<String>,
    },
    ReportRoom {
        request_id: RequestId,
        room_id: String,
        reason: String,
    },
}

impl fmt::Debug for RoomCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateRoom {
                request_id,
                options,
                ..
            } => formatter
                .debug_struct("CreateRoom")
                .field("request_id", request_id)
                .field("name", &"RoomName(..)")
                .field("encrypted", &options.encrypted)
                .field("visibility", &options.visibility)
                .field("has_topic", &options.topic.is_some())
                .field("has_alias_localpart", &options.alias_localpart.is_some())
                .field("has_parent_space", &options.parent_space.is_some())
                .finish(),
            Self::CreatePublicDirectoryRoom { request_id, .. } => formatter
                .debug_struct("CreatePublicDirectoryRoom")
                .field("request_id", request_id)
                .field("name", &"RoomName(..)")
                .field("alias_localpart", &"RoomAliasLocalpart(..)")
                .finish(),
            Self::CreateSpace { request_id, .. } => formatter
                .debug_struct("CreateSpace")
                .field("request_id", request_id)
                .field("name", &"RoomName(..)")
                .finish(),
            Self::SetSpaceChild { request_id, .. } => formatter
                .debug_struct("SetSpaceChild")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("child_room_id", &"RoomId(..)")
                .field("via_server", &"ServerName(..)")
                .finish(),
            Self::InviteUser { request_id, .. } => formatter
                .debug_struct("InviteUser")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::InviteTargets {
                request_id,
                user_ids,
                scope,
                ..
            } => formatter
                .debug_struct("InviteTargets")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("user_count", &user_ids.len())
                .field("scope", scope)
                .finish(),
            Self::AcceptInvite { request_id, .. } => formatter
                .debug_struct("AcceptInvite")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::DeclineInvite { request_id, .. } => formatter
                .debug_struct("DeclineInvite")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::StartDirectMessage { request_id, .. } => formatter
                .debug_struct("StartDirectMessage")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::JoinRoom { request_id, .. } => formatter
                .debug_struct("JoinRoom")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::LeaveRoom { request_id, .. } => formatter
                .debug_struct("LeaveRoom")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::ForgetRoom { request_id, .. } => formatter
                .debug_struct("ForgetRoom")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::SetTag {
                request_id,
                tag,
                order,
                ..
            } => formatter
                .debug_struct("SetTag")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("tag", tag)
                .field("order", order)
                .finish(),
            Self::RemoveTag {
                request_id, tag, ..
            } => formatter
                .debug_struct("RemoveTag")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("tag", tag)
                .finish(),
            Self::PinEvent { request_id, .. } => formatter
                .debug_struct("PinEvent")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::UnpinEvent { request_id, .. } => formatter
                .debug_struct("UnpinEvent")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::QueryDirectory { request_id, query } => formatter
                .debug_struct("QueryDirectory")
                .field("request_id", request_id)
                .field("term", &query.term.as_ref().map(|_| "DirectoryQuery(..)"))
                .field(
                    "server_name",
                    &query.server_name.as_ref().map(|_| "ServerName(..)"),
                )
                .field("limit", &query.limit)
                .field("since", &query.since.as_ref().map(|_| "PageToken(..)"))
                .finish(),
            Self::JoinDirectoryRoom { request_id, .. } => formatter
                .debug_struct("JoinDirectoryRoom")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &"ServerName(..)")
                .finish(),
            Self::LoadRoomSettings { request_id, .. } => formatter
                .debug_struct("LoadRoomSettings")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::ReshareRoomKey { request_id, .. } => formatter
                .debug_struct("ReshareRoomKey")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::UpdateRoomSetting {
                request_id, change, ..
            } => formatter
                .debug_struct("UpdateRoomSetting")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("change", change)
                .finish(),
            Self::ModerateRoomMember {
                request_id, action, ..
            } => formatter
                .debug_struct("ModerateRoomMember")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("action", action)
                .field("reason", &"ModerationReason(..)")
                .finish(),
            Self::UpdateRoomMemberRole {
                request_id,
                power_level,
                ..
            } => formatter
                .debug_struct("UpdateRoomMemberRole")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("power_level", power_level)
                .finish(),
            Self::SelectSpace {
                request_id,
                space_id,
            } => formatter
                .debug_struct("SelectSpace")
                .field("request_id", request_id)
                .field("space_id", &space_id.as_ref().map(|_| "RoomId(..)"))
                .finish(),
            Self::ReorderSpaces { request_id, .. } => formatter
                .debug_struct("ReorderSpaces")
                .field("request_id", request_id)
                .field("space_ids", &"Vec<RoomId>(..)")
                .finish(),
            Self::SelectRoom { request_id, .. } => formatter
                .debug_struct("SelectRoom")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::MarkRoomAsRead {
                request_id,
                room_id: _,
                ..
            } => formatter
                .debug_struct("MarkRoomAsRead")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::MarkRoomAsUnread {
                request_id,
                room_id: _,
                unread,
            } => formatter
                .debug_struct("MarkRoomAsUnread")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("unread", unread)
                .finish(),
            Self::SetRoomNotificationMode {
                request_id,
                room_id: _,
                mode,
            } => formatter
                .debug_struct("SetRoomNotificationMode")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("mode", mode)
                .finish(),
            Self::ReportContent { request_id, .. } => formatter
                .debug_struct("ReportContent")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .field("reason", &"ReportReason(..)")
                .finish(),
            Self::ReportRoom { request_id, .. } => formatter
                .debug_struct("ReportRoom")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("reason", &"ReportReason(..)")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UploadMediaKind {
    Image {
        width: Option<u64>,
        height: Option<u64>,
    },
    File,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadDimensions {
    pub width: u64,
    pub height: u64,
}

impl ImageUploadDimensions {
    pub fn long_edge(self) -> u64 {
        self.width.max(self.height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadCompressionPolicy {
    pub threshold_bytes: u64,
    pub threshold_long_edge: u64,
    pub target_long_edge: u64,
    pub quality_percent: u8,
}

impl Default for ImageUploadCompressionPolicy {
    fn default() -> Self {
        Self {
            threshold_bytes: 1_048_576,
            threshold_long_edge: 2560,
            target_long_edge: 2048,
            quality_percent: 82,
        }
    }
}

impl ImageUploadCompressionPolicy {
    pub fn should_skip(self, info: &ImageUploadVariantInfo) -> bool {
        if info.byte_count > self.threshold_bytes {
            return false;
        }
        match info.dimensions {
            Some(dimensions) => dimensions.long_edge() <= self.threshold_long_edge,
            None => true,
        }
    }

    pub fn target_dimensions_for(self, dimensions: ImageUploadDimensions) -> ImageUploadDimensions {
        let long_edge = dimensions.long_edge();
        if long_edge == 0 || long_edge <= self.target_long_edge {
            return dimensions;
        }

        ImageUploadDimensions {
            width: scale_dimension(dimensions.width, long_edge, self.target_long_edge),
            height: scale_dimension(dimensions.height, long_edge, self.target_long_edge),
        }
    }
}

fn scale_dimension(value: u64, source_long_edge: u64, target_long_edge: u64) -> u64 {
    if value == 0 || source_long_edge == 0 {
        return value;
    }
    let numerator = u128::from(value) * u128::from(target_long_edge);
    let denominator = u128::from(source_long_edge);
    let rounded = (numerator + (denominator / 2)) / denominator;
    u64::try_from(rounded.max(1)).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadVariantInfo {
    pub mime_type: String,
    pub byte_count: u64,
    pub dimensions: Option<ImageUploadDimensions>,
}

impl ImageUploadVariantInfo {
    pub fn selected(
        mime_type: String,
        byte_count: u64,
        dimensions: Option<ImageUploadDimensions>,
    ) -> Self {
        Self {
            mime_type,
            byte_count,
            dimensions,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ImageUploadVariantKind {
    Original,
    Compressed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadCompressionState {
    pub mode: ImageUploadCompressionMode,
    pub policy: ImageUploadCompressionPolicy,
    pub original: ImageUploadVariantInfo,
    pub selected: ImageUploadVariantInfo,
    pub selected_variant: ImageUploadVariantKind,
    pub skipped_small_image: bool,
    pub metadata_stripped: bool,
    pub thumbnail_refreshed: bool,
}

impl ImageUploadCompressionState {
    pub fn original(
        mode: ImageUploadCompressionMode,
        mime_type: String,
        byte_count: u64,
        dimensions: Option<ImageUploadDimensions>,
    ) -> Self {
        let policy = ImageUploadCompressionPolicy::default();
        let original = ImageUploadVariantInfo::selected(mime_type, byte_count, dimensions);
        let skipped_small_image = policy.should_skip(&original);
        Self {
            mode,
            policy,
            original: original.clone(),
            selected: original,
            selected_variant: ImageUploadVariantKind::Original,
            skipped_small_image,
            metadata_stripped: false,
            thumbnail_refreshed: false,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UploadMediaThumbnail {
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub width: u64,
    pub height: u64,
}

impl fmt::Debug for UploadMediaThumbnail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadMediaThumbnail")
            .field("mime_type", &self.mime_type)
            .field("bytes", &"ThumbnailBytes(..)")
            .field("bytes_len", &self.bytes.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct UploadMediaRequest {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub kind: UploadMediaKind,
    pub compression: Option<ImageUploadCompressionState>,
    pub thumbnail: Option<UploadMediaThumbnail>,
    pub caption: Option<FormattedMessageDraft>,
}

impl fmt::Debug for UploadMediaRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadMediaRequest")
            .field("filename", &"MediaFilename(..)")
            .field("mime_type", &self.mime_type)
            .field("bytes", &"MediaBytes(..)")
            .field("bytes_len", &self.bytes.len())
            .field("kind", &self.kind)
            .field("compression", &self.compression)
            .field("thumbnail", &self.thumbnail)
            .field(
                "caption",
                &self.caption.as_ref().map(|_| "MediaCaption(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaDownloadSelection {
    File,
    Thumbnail { width: u64, height: u64 },
}

pub enum TimelineCommand {
    Subscribe {
        request_id: RequestId,
        key: TimelineKey,
    },
    EnsureSubscribed {
        request_id: RequestId,
        key: TimelineKey,
        replay_existing: bool,
    },
    ReplaySubscribed {
        request_id: RequestId,
    },
    Unsubscribe {
        request_id: RequestId,
        key: TimelineKey,
    },
    Paginate {
        request_id: RequestId,
        key: TimelineKey,
        direction: crate::event::PaginationDirection,
        event_count: u16,
    },
    CancelPagination {
        request_id: RequestId,
        key: TimelineKey,
    },
    CancelLinkPreviews {
        request_id: RequestId,
        key: TimelineKey,
    },
    RestoreTimelineAnchor {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        max_batches: u16,
        event_count: u16,
    },
    ObserveViewport {
        request_id: RequestId,
        key: TimelineKey,
        observation: TimelineViewportObservation,
    },
    RepairGaps {
        request_id: RequestId,
        key: TimelineKey,
    },
    SendText {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        body: String,
        mentions: MentionIntent,
    },
    SubmitText {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        submission_id: SubmissionId,
        key: TimelineKey,
        transaction_id: String,
        body: String,
        mentions: MentionIntent,
        draft_revision: u64,
    },
    SendReply {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        in_reply_to_event_id: String,
        body: String,
        mentions: MentionIntent,
    },
    SubmitReply {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        submission_id: SubmissionId,
        key: TimelineKey,
        transaction_id: String,
        in_reply_to_event_id: String,
        body: String,
        mentions: MentionIntent,
        draft_revision: u64,
    },
    ForwardMessage {
        request_id: RequestId,
        key: TimelineKey,
        source_event_id: String,
        destination_room_id: String,
        transaction_id: String,
    },
    LoadMessageSource {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    RequestRoomKey {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    RetrySend {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
    },
    CancelSend {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
    },
    UploadAndSendMedia {
        request_id: RequestId,
        expected_account: koushi_key::SessionKeyId,
        key: TimelineKey,
        transaction_id: String,
        request: UploadMediaRequest,
    },
    DownloadMedia {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        selection: MediaDownloadSelection,
    },
    EditText {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        body: String,
    },
    Redact {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    ToggleReaction {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        reaction_key: String,
    },
    SendReaction {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        reaction_key: String,
    },
    RedactReaction {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        reaction_key: String,
        reaction_event_id: String,
    },
    SendReadReceipt {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    SetFullyRead {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    SetTyping {
        request_id: RequestId,
        key: TimelineKey,
        is_typing: bool,
    },
    LoadLinkPreviews {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    HideLinkPreview {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    BroadcastLinkPreviewPolicy {
        unencrypted_global_enabled: bool,
        encrypted_global_enabled: bool,
        room_overrides: std::collections::BTreeMap<String, bool>,
    },
}

impl TimelineCommand {
    /// Complete account owner captured by composer-affecting commands.
    ///
    /// AppActor and AccountActor both revalidate this immediately before
    /// routing so an account switch cannot redirect an already-issued send.
    pub fn composer_account_fence(&self) -> Option<(RequestId, &koushi_key::SessionKeyId)> {
        match self {
            Self::SubmitText {
                request_id,
                expected_account,
                ..
            }
            | Self::SubmitReply {
                request_id,
                expected_account,
                ..
            }
            | Self::UploadAndSendMedia {
                request_id,
                expected_account,
                ..
            } => Some((*request_id, expected_account)),
            _ => None,
        }
    }
}

// Message bodies and reaction keys are visible UI state but must not reach
// logs through Debug (spec: "SendText and EditText redact body in Debug and
// errors").
impl fmt::Debug for TimelineCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Subscribe { request_id, key } => formatter
                .debug_struct("Subscribe")
                .field("request_id", request_id)
                .field("key", key)
                .finish(),
            Self::EnsureSubscribed {
                request_id,
                key,
                replay_existing,
            } => formatter
                .debug_struct("EnsureSubscribed")
                .field("request_id", request_id)
                .field("key", key)
                .field("replay_existing", replay_existing)
                .finish(),
            Self::ReplaySubscribed { request_id } => formatter
                .debug_struct("ReplaySubscribed")
                .field("request_id", request_id)
                .finish(),
            Self::Unsubscribe { request_id, key } => formatter
                .debug_struct("Unsubscribe")
                .field("request_id", request_id)
                .field("key", key)
                .finish(),
            Self::Paginate {
                request_id,
                key,
                direction,
                event_count,
            } => formatter
                .debug_struct("Paginate")
                .field("request_id", request_id)
                .field("key", key)
                .field("direction", direction)
                .field("event_count", event_count)
                .finish(),
            Self::CancelPagination { request_id, key } => formatter
                .debug_struct("CancelPagination")
                .field("request_id", request_id)
                .field("key", key)
                .finish(),
            Self::CancelLinkPreviews { request_id, key } => formatter
                .debug_struct("CancelLinkPreviews")
                .field("request_id", request_id)
                .field("key", key)
                .finish(),
            Self::RestoreTimelineAnchor {
                request_id,
                max_batches,
                event_count,
                ..
            } => formatter
                .debug_struct("RestoreTimelineAnchor")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .field("max_batches", max_batches)
                .field("event_count", event_count)
                .finish(),
            Self::ObserveViewport { request_id, .. } => formatter
                .debug_struct("ObserveViewport")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("first_visible_event_id", &"EventId(..)")
                .field("last_visible_event_id", &"EventId(..)")
                .field("at_bottom", &"ViewportFact(..)")
                .finish(),
            Self::RepairGaps { request_id, .. } => formatter
                .debug_struct("RepairGaps")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .finish(),
            Self::SendText {
                request_id,
                key,
                transaction_id,
                ..
            } => formatter
                .debug_struct("SendText")
                .field("request_id", request_id)
                .field("key", key)
                .field("transaction_id", transaction_id)
                .field("body", &"MessageBody(..)")
                .field("mentions", &"MentionIntent(..)")
                .finish(),
            Self::SendReply {
                request_id,
                key,
                transaction_id,
                ..
            } => formatter
                .debug_struct("SendReply")
                .field("request_id", request_id)
                .field("key", key)
                .field("transaction_id", transaction_id)
                .field("in_reply_to_event_id", &"EventId(..)")
                .field("body", &"MessageBody(..)")
                .field("mentions", &"MentionIntent(..)")
                .finish(),
            Self::SubmitText {
                request_id,
                submission_id,
                transaction_id,
                ..
            } => formatter
                .debug_struct("SubmitText")
                .field("request_id", request_id)
                .field("submission_id", submission_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", transaction_id)
                .field("body", &"MessageBody(..)")
                .field("mentions", &"MentionIntent(..)")
                .finish(),
            Self::SubmitReply {
                request_id,
                submission_id,
                transaction_id,
                ..
            } => formatter
                .debug_struct("SubmitReply")
                .field("request_id", request_id)
                .field("submission_id", submission_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", transaction_id)
                .field("in_reply_to_event_id", &"EventId(..)")
                .field("body", &"MessageBody(..)")
                .field("mentions", &"MentionIntent(..)")
                .finish(),
            Self::ForwardMessage { request_id, .. } => formatter
                .debug_struct("ForwardMessage")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("source_event_id", &"EventId(..)")
                .field("destination_room_id", &"RoomId(..)")
                .field("transaction_id", &"TransactionId(..)")
                .finish(),
            Self::LoadMessageSource { request_id, .. } => formatter
                .debug_struct("LoadMessageSource")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::RequestRoomKey { request_id, .. } => formatter
                .debug_struct("RequestRoomKey")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::RetrySend { request_id, .. } => formatter
                .debug_struct("RetrySend")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", &"TransactionId(..)")
                .finish(),
            Self::CancelSend { request_id, .. } => formatter
                .debug_struct("CancelSend")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", &"TransactionId(..)")
                .finish(),
            Self::UploadAndSendMedia {
                request_id,
                key,
                transaction_id,
                request,
                ..
            } => formatter
                .debug_struct("UploadAndSendMedia")
                .field("request_id", request_id)
                .field("key", key)
                .field("transaction_id", transaction_id)
                .field("mime_type", &request.mime_type)
                .field("kind", &request.kind)
                .field("filename", &"MediaFilename(..)")
                .field("bytes", &"MediaBytes(..)")
                .field("compression", &request.compression)
                .field("thumbnail", &request.thumbnail)
                .field(
                    "caption",
                    &request.caption.as_ref().map(|_| "MediaCaption(..)"),
                )
                .finish(),
            Self::DownloadMedia {
                request_id,
                key,
                selection,
                ..
            } => formatter
                .debug_struct("DownloadMedia")
                .field("request_id", request_id)
                .field("key", key)
                .field("event_id", &"EventId(..)")
                .field("selection", selection)
                .finish(),
            Self::EditText {
                request_id,
                key,
                event_id,
                ..
            } => formatter
                .debug_struct("EditText")
                .field("request_id", request_id)
                .field("key", key)
                .field("event_id", event_id)
                .field("body", &"MessageBody(..)")
                .finish(),
            Self::Redact {
                request_id, key, ..
            } => formatter
                .debug_struct("Redact")
                .field("request_id", request_id)
                .field("key", key)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::ToggleReaction {
                request_id, key, ..
            } => formatter
                .debug_struct("ToggleReaction")
                .field("request_id", request_id)
                .field("key", key)
                .field("event_id", &"EventId(..)")
                .field("reaction_key", &"ReactionKey(..)")
                .finish(),
            Self::SendReaction {
                request_id, key, ..
            } => formatter
                .debug_struct("SendReaction")
                .field("request_id", request_id)
                .field("key", key)
                .field("event_id", &"EventId(..)")
                .field("reaction_key", &"ReactionKey(..)")
                .finish(),
            Self::RedactReaction {
                request_id, key, ..
            } => formatter
                .debug_struct("RedactReaction")
                .field("request_id", request_id)
                .field("key", key)
                .field("event_id", &"EventId(..)")
                .field("reaction_key", &"ReactionKey(..)")
                .field("reaction_event_id", &"EventId(..)")
                .finish(),
            Self::SendReadReceipt { request_id, .. } => formatter
                .debug_struct("SendReadReceipt")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::SetFullyRead { request_id, .. } => formatter
                .debug_struct("SetFullyRead")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::SetTyping {
                request_id,
                is_typing,
                ..
            } => formatter
                .debug_struct("SetTyping")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("is_typing", is_typing)
                .finish(),
            Self::LoadLinkPreviews { request_id, .. } => formatter
                .debug_struct("LoadLinkPreviews")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::HideLinkPreview { request_id, .. } => formatter
                .debug_struct("HideLinkPreview")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::BroadcastLinkPreviewPolicy {
                unencrypted_global_enabled,
                encrypted_global_enabled,
                room_overrides,
            } => formatter
                .debug_struct("BroadcastLinkPreviewPolicy")
                .field("unencrypted_global_enabled", unencrypted_global_enabled)
                .field("encrypted_global_enabled", encrypted_global_enabled)
                .field("room_override_count", &room_overrides.len())
                .finish(),
        }
    }
}

pub enum SearchCommand {
    Query {
        request_id: RequestId,
        query: String,
        scope: SearchScope,
    },
    Attachments {
        request_id: RequestId,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    StartHistoryCrawl {
        request_id: RequestId,
        room_id: String,
        settings: koushi_state::SearchCrawlerSettings,
    },
    StopHistoryCrawl {
        request_id: RequestId,
        room_id: String,
    },
}

#[derive(Clone, Debug)]
pub enum ThreadsListCommand {
    Open {
        request_id: RequestId,
        room_id: String,
    },
    Close {
        request_id: RequestId,
    },
    Paginate {
        request_id: RequestId,
        room_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchScope {
    AllRooms,
    CurrentRoom { room_id: String },
    CurrentSpace { space_id: String },
    Dms,
}

// Search queries can quote message content; redact like bodies.
impl fmt::Debug for SearchCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Query {
                request_id, scope, ..
            } => formatter
                .debug_struct("Query")
                .field("request_id", request_id)
                .field("query", &"SearchQuery(..)")
                .field("scope", scope)
                .finish(),
            Self::Attachments {
                request_id,
                scope,
                filter,
                sort,
            } => formatter
                .debug_struct("Attachments")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .finish(),
            Self::StartHistoryCrawl {
                request_id,
                room_id: _,
                settings,
            } => formatter
                .debug_struct("StartHistoryCrawl")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("settings", settings)
                .finish(),
            Self::StopHistoryCrawl {
                request_id,
                room_id: _,
            } => formatter
                .debug_struct("StopHistoryCrawl")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use koushi_state::{
        ImageUploadCompressionMode, MentionIntent, MentionTarget, NativeAttentionCandidate,
        NativeAttentionCapabilities, NativeAttentionCapability, NativeAttentionDispatchState,
        NativeAttentionState, NativeAttentionSummary, NativeAttentionSuppressionReason,
        RoomAttentionKind,
    };

    use super::*;

    fn fake_rid(seq: u64) -> RequestId {
        RequestId {
            connection_id: crate::ids::RuntimeConnectionId(999),
            sequence: seq,
        }
    }

    fn test_session_key() -> koushi_key::SessionKeyId {
        koushi_key::SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@a:test".to_owned(),
            device_id: "DEVICE".to_owned(),
        }
    }

    #[test]
    fn soft_logout_reauth_is_allowed_past_ready_session_gate() {
        let command = CoreCommand::Account(AccountCommand::SoftLogoutReauth {
            request_id: fake_rid(73),
            password: koushi_state::AuthSecret::new("synthetic-password"),
        });

        assert!(!command.requires_ready_session());
    }

    #[cfg(feature = "qa-bin")]
    #[test]
    fn qa_device_key_refresh_is_ready_gated_correlated_and_redacted() {
        let request_id = fake_rid(74);
        let (acknowledged, _ack) = tokio::sync::oneshot::channel();
        let command = CoreCommand::Account(AccountCommand::QaRefreshDeviceKeysAndAssertKnown {
            request_id,
            target: VerificationTarget {
                user_id: "@private-user:example.invalid".to_owned(),
                device_id: "PRIVATEDEVICE".to_owned(),
            },
            acknowledged,
        });

        assert_eq!(command.request_id(), request_id);
        assert!(command.requires_ready_session());
        let debug = format!("{command:?}");
        assert!(
            debug.contains("QaRefreshDeviceKeysAndAssertKnown"),
            "{debug}"
        );
        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains("private-user"), "{debug}");
        assert!(!debug.contains("PRIVATEDEVICE"), "{debug}");
    }

    #[test]
    fn send_text_debug_redacts_body_and_mentions() {
        let command = TimelineCommand::SendText {
            request_id: fake_rid(6),
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
            transaction_id: "txn-text".to_owned(),
            body: "secret text body".to_owned(),
            mentions: MentionIntent {
                targets: vec![MentionTarget::User {
                    user_id: "@alice:example.test".to_owned(),
                    display_label: "Alice".to_owned(),
                }],
            },
        };

        let debug = format!("{command:?}");
        assert!(debug.contains("SendText"), "{debug}");
        assert!(debug.contains("txn-text"), "{debug}");
        assert!(!debug.contains("secret text body"), "{debug}");
        assert!(!debug.contains("@alice:example.test"), "{debug}");
        assert!(!debug.contains("Alice"), "{debug}");
    }

    #[test]
    fn send_reply_debug_redacts_body_and_event_ids() {
        let command = TimelineCommand::SendReply {
            request_id: fake_rid(7),
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
            transaction_id: "txn-reply".to_owned(),
            in_reply_to_event_id: "$event:test".to_owned(),
            body: "secret reply body".to_owned(),
            mentions: MentionIntent::default(),
        };

        let debug = format!("{command:?}");
        assert!(debug.contains("SendReply"), "{debug}");
        assert!(debug.contains("txn-reply"), "{debug}");
        assert!(!debug.contains("secret reply body"), "{debug}");
        assert!(!debug.contains("$event:test"), "{debug}");
    }

    #[test]
    fn forward_message_debug_redacts_source_destination_and_transaction() {
        let request_id = fake_rid(71);
        let command = TimelineCommand::ForwardMessage {
            request_id,
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
            source_event_id: "$source-event:test".to_owned(),
            destination_room_id: "!destination-room:test".to_owned(),
            transaction_id: "txn-forward-private".to_owned(),
        };

        assert_eq!(CoreCommand::Timeline(command).request_id(), request_id);

        let command = TimelineCommand::ForwardMessage {
            request_id,
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
            source_event_id: "$source-event:test".to_owned(),
            destination_room_id: "!destination-room:test".to_owned(),
            transaction_id: "txn-forward-private".to_owned(),
        };
        let debug = format!("{command:?}");
        assert!(debug.contains("ForwardMessage"), "{debug}");
        assert!(debug.contains("TimelineKey(..)"), "{debug}");
        assert!(debug.contains("EventId(..)"), "{debug}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(debug.contains("TransactionId(..)"), "{debug}");
        assert!(!debug.contains("@a:test"), "{debug}");
        assert!(!debug.contains("!source-room:test"), "{debug}");
        assert!(!debug.contains("$source-event:test"), "{debug}");
        assert!(!debug.contains("!destination-room:test"), "{debug}");
        assert!(!debug.contains("txn-forward-private"), "{debug}");
    }

    #[test]
    fn load_message_source_debug_redacts_timeline_key_and_event_id() {
        let request_id = fake_rid(72);
        let command = TimelineCommand::LoadMessageSource {
            request_id,
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
            event_id: "$source-event:test".to_owned(),
        };

        assert_eq!(CoreCommand::Timeline(command).request_id(), request_id);

        let command = TimelineCommand::LoadMessageSource {
            request_id,
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
            event_id: "$source-event:test".to_owned(),
        };
        let debug = format!("{command:?}");
        assert!(debug.contains("LoadMessageSource"), "{debug}");
        assert!(debug.contains("TimelineKey(..)"), "{debug}");
        assert!(debug.contains("EventId(..)"), "{debug}");
        assert!(!debug.contains("@a:test"), "{debug}");
        assert!(!debug.contains("!source-room:test"), "{debug}");
        assert!(!debug.contains("$source-event:test"), "{debug}");
    }

    #[test]
    fn upload_media_debug_redacts_filename_caption_and_bytes() {
        let dimensions = ImageUploadDimensions {
            width: 1200,
            height: 900,
        };
        let compression = ImageUploadCompressionState {
            mode: ImageUploadCompressionMode::Always,
            policy: ImageUploadCompressionPolicy::default(),
            original: ImageUploadVariantInfo {
                mime_type: "image/jpeg".to_owned(),
                byte_count: 3_200_000,
                dimensions: Some(ImageUploadDimensions {
                    width: 4032,
                    height: 3024,
                }),
            },
            selected: ImageUploadVariantInfo {
                mime_type: "image/jpeg".to_owned(),
                byte_count: 128_000,
                dimensions: Some(dimensions),
            },
            selected_variant: ImageUploadVariantKind::Compressed,
            skipped_small_image: false,
            metadata_stripped: true,
            thumbnail_refreshed: true,
        };
        let command = TimelineCommand::UploadAndSendMedia {
            request_id: fake_rid(8),
            expected_account: test_session_key(),
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
            transaction_id: "txn-media".to_owned(),
            request: UploadMediaRequest {
                filename: "private-fixture-name.png".to_owned(),
                mime_type: "image/png".to_owned(),
                bytes: vec![1, 2, 3, 4],
                kind: UploadMediaKind::Image {
                    width: Some(2),
                    height: Some(2),
                },
                compression: Some(compression),
                thumbnail: Some(UploadMediaThumbnail {
                    mime_type: "image/jpeg".to_owned(),
                    bytes: vec![9, 8, 7, 6],
                    width: 320,
                    height: 240,
                }),
                caption: Some(koushi_state::build_formatted_message_draft(
                    "private caption",
                    MentionIntent::default(),
                )),
            },
        };

        let debug = format!("{command:?}");
        assert!(debug.contains("UploadAndSendMedia"), "{debug}");
        assert!(debug.contains("txn-media"), "{debug}");
        assert!(debug.contains("image/png"), "{debug}");
        assert!(debug.contains("Compressed"), "{debug}");
        assert!(debug.contains("thumbnail"), "{debug}");
        assert!(!debug.contains("private-fixture-name.png"), "{debug}");
        assert!(!debug.contains("private caption"), "{debug}");
        assert!(!debug.contains("1, 2, 3, 4"), "{debug}");
        assert!(!debug.contains("9, 8, 7, 6"), "{debug}");
    }

    #[test]
    fn image_upload_compression_policy_preserves_aspect_ratio_and_skips_small_images() {
        let policy = ImageUploadCompressionPolicy::default();

        assert_eq!(policy.threshold_bytes, 1_048_576);
        assert_eq!(policy.threshold_long_edge, 2560);
        assert_eq!(policy.target_long_edge, 2048);
        assert_eq!(policy.quality_percent, 82);
        assert_eq!(
            policy.target_dimensions_for(ImageUploadDimensions {
                width: 4032,
                height: 3024
            }),
            ImageUploadDimensions {
                width: 2048,
                height: 1536
            }
        );
        assert_eq!(
            policy.target_dimensions_for(ImageUploadDimensions {
                width: 1024,
                height: 768
            }),
            ImageUploadDimensions {
                width: 1024,
                height: 768
            }
        );

        let small = ImageUploadVariantInfo {
            mime_type: "image/png".to_owned(),
            byte_count: 64_000,
            dimensions: Some(ImageUploadDimensions {
                width: 800,
                height: 600,
            }),
        };
        let large_by_size = ImageUploadVariantInfo {
            mime_type: "image/png".to_owned(),
            byte_count: 2_000_000,
            dimensions: Some(ImageUploadDimensions {
                width: 800,
                height: 600,
            }),
        };
        let large_by_dimension = ImageUploadVariantInfo {
            mime_type: "image/png".to_owned(),
            byte_count: 64_000,
            dimensions: Some(ImageUploadDimensions {
                width: 4096,
                height: 512,
            }),
        };

        assert!(policy.should_skip(&small));
        assert!(!policy.should_skip(&large_by_size));
        assert!(!policy.should_skip(&large_by_dimension));
    }

    #[test]
    fn retry_and_cancel_send_debug_redacts_timeline_key_and_transaction_id() {
        let key = TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test");
        let retry = TimelineCommand::RetrySend {
            request_id: fake_rid(9),
            key: key.clone(),
            transaction_id: "txn-private".to_owned(),
        };
        let cancel = TimelineCommand::CancelSend {
            request_id: fake_rid(10),
            key,
            transaction_id: "txn-private".to_owned(),
        };

        for debug in [format!("{retry:?}"), format!("{cancel:?}")] {
            assert!(!debug.contains("!room:test"), "{debug}");
            assert!(!debug.contains("@a:test"), "{debug}");
            assert!(!debug.contains("txn-private"), "{debug}");
            assert!(debug.contains("TransactionId(..)"), "{debug}");
        }
    }

    #[test]
    fn pin_event_debug_redacts_room_and_event_ids() {
        let pin = RoomCommand::PinEvent {
            request_id: fake_rid(11),
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        };
        let unpin = RoomCommand::UnpinEvent {
            request_id: fake_rid(12),
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        };

        for debug in [format!("{pin:?}"), format!("{unpin:?}")] {
            assert!(debug.contains("RoomId(..)"), "{debug}");
            assert!(debug.contains("EventId(..)"), "{debug}");
            assert!(!debug.contains("!room:example.invalid"), "{debug}");
            assert!(!debug.contains("$event:example.invalid"), "{debug}");
        }
    }

    #[test]
    fn set_room_notification_mode_debug_redacts_room_id() {
        let command = RoomCommand::SetRoomNotificationMode {
            request_id: fake_rid(13),
            room_id: "!room:example.invalid".to_owned(),
            mode: koushi_state::RoomNotificationMode::Mute,
        };
        let debug = format!("{command:?}");
        assert!(debug.contains("SetRoomNotificationMode"), "{debug}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(!debug.contains("!room:example.invalid"), "{debug}");
    }

    #[test]
    fn set_room_url_preview_override_debug_redacts_room_id() {
        let command = AppCommand::SetRoomUrlPreviewOverride {
            request_id: fake_rid(14),
            room_id: "!room:example.invalid".to_owned(),
            enabled: false,
        };
        let debug = format!("{command:?}");
        assert!(debug.contains("SetRoomUrlPreviewOverride"), "{debug}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(debug.contains("enabled"), "{debug}");
        assert!(!debug.contains("!room:example.invalid"), "{debug}");
    }

    #[test]
    fn directory_commands_debug_redacts_query_alias_and_server() {
        let query = RoomCommand::QueryDirectory {
            request_id: fake_rid(15),
            query: DirectoryQuery {
                term: Some("private search".to_owned()),
                server_name: Some("example.invalid".to_owned()),
                limit: Some(10),
                since: Some("opaque-page-token".to_owned()),
            },
        };
        let join_request_id = fake_rid(14);
        let join = RoomCommand::JoinDirectoryRoom {
            request_id: join_request_id,
            alias: "#private-room:example.invalid".to_owned(),
            via_server: Some("example.invalid".to_owned()),
        };
        let create_request_id = fake_rid(15);
        let create_public = RoomCommand::CreatePublicDirectoryRoom {
            request_id: create_request_id,
            name: "Private Public Room Name".to_owned(),
            alias_localpart: "private-public-alias".to_owned(),
        };

        assert_eq!(
            CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
                request_id: join_request_id,
                alias: "#private-room:example.invalid".to_owned(),
                via_server: Some("example.invalid".to_owned()),
            })
            .request_id(),
            join_request_id
        );
        assert_eq!(
            CoreCommand::Room(RoomCommand::CreatePublicDirectoryRoom {
                request_id: create_request_id,
                name: "Private Public Room Name".to_owned(),
                alias_localpart: "private-public-alias".to_owned(),
            })
            .request_id(),
            create_request_id
        );
        for debug in [
            format!("{query:?}"),
            format!("{join:?}"),
            format!("{create_public:?}"),
        ] {
            assert!(!debug.contains("private search"), "{debug}");
            assert!(!debug.contains("#private-room:example.invalid"), "{debug}");
            assert!(!debug.contains("Private Public Room Name"), "{debug}");
            assert!(!debug.contains("private-public-alias"), "{debug}");
            assert!(!debug.contains("example.invalid"), "{debug}");
            assert!(!debug.contains("opaque-page-token"), "{debug}");
        }
    }

    #[test]
    fn room_management_commands_debug_redacts_room_user_and_settings_values() {
        use koushi_state::{RoomJoinRule, RoomModerationAction, RoomSettingChange};

        let load_request_id = fake_rid(16);
        let load = RoomCommand::LoadRoomSettings {
            request_id: load_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
        };
        let update_request_id = fake_rid(17);
        let update = RoomCommand::UpdateRoomSetting {
            request_id: update_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            change: RoomSettingChange::Name(Some("Private Room Name".to_owned())),
        };
        let moderation_request_id = fake_rid(18);
        let moderation = RoomCommand::ModerateRoomMember {
            request_id: moderation_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            target_user_id: "@private-target:example.invalid".to_owned(),
            action: RoomModerationAction::Ban,
            reason: Some("Private moderation reason".to_owned()),
        };
        let role_request_id = fake_rid(19);
        let role = RoomCommand::UpdateRoomMemberRole {
            request_id: role_request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            target_user_id: "@private-target:example.invalid".to_owned(),
            power_level: 50,
        };

        assert_eq!(CoreCommand::Room(load).request_id(), load_request_id);
        assert_eq!(
            CoreCommand::Room(RoomCommand::UpdateRoomSetting {
                request_id: update_request_id,
                room_id: "!private-room:example.invalid".to_owned(),
                change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
            })
            .request_id(),
            update_request_id
        );
        assert_eq!(
            CoreCommand::Room(RoomCommand::ModerateRoomMember {
                request_id: moderation_request_id,
                room_id: "!private-room:example.invalid".to_owned(),
                target_user_id: "@private-target:example.invalid".to_owned(),
                action: RoomModerationAction::Kick,
                reason: None,
            })
            .request_id(),
            moderation_request_id
        );
        assert_eq!(
            CoreCommand::Room(RoomCommand::UpdateRoomMemberRole {
                request_id: role_request_id,
                room_id: "!private-room:example.invalid".to_owned(),
                target_user_id: "@private-target:example.invalid".to_owned(),
                power_level: 50,
            })
            .request_id(),
            role_request_id
        );

        for debug in [
            format!(
                "{:?}",
                RoomCommand::LoadRoomSettings {
                    request_id: fake_rid(20),
                    room_id: "!private-room:example.invalid".to_owned(),
                }
            ),
            format!("{update:?}"),
            format!("{moderation:?}"),
            format!("{role:?}"),
        ] {
            assert!(debug.contains("RoomId(..)"), "{debug}");
            assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
            assert!(
                !debug.contains("@private-target:example.invalid"),
                "{debug}"
            );
            assert!(!debug.contains("Private Room Name"), "{debug}");
            assert!(!debug.contains("Private moderation reason"), "{debug}");
        }
    }

    #[test]
    fn activity_commands_debug_redacts_targets_and_carry_request_ids() {
        use koushi_state::{ActivityMarkReadTarget, ActivityTab};

        let set_tab_request_id = fake_rid(21);
        let set_tab = AppCommand::SetActivityTab {
            request_id: set_tab_request_id,
            tab: ActivityTab::Unread,
        };
        let paginate_request_id = fake_rid(22);
        let paginate = AppCommand::PaginateActivity {
            request_id: paginate_request_id,
            tab: ActivityTab::Recent,
            cursor: Some("private-page-token".to_owned()),
        };
        let mark_request_id = fake_rid(23);
        let mark = AppCommand::MarkActivityRead {
            request_id: mark_request_id,
            target: ActivityMarkReadTarget::Room {
                room_id: "!private-room:example.invalid".to_owned(),
                up_to_event_id: "$private-event:example.invalid".to_owned(),
            },
        };

        assert_eq!(CoreCommand::App(set_tab).request_id(), set_tab_request_id);
        assert_eq!(CoreCommand::App(paginate).request_id(), paginate_request_id);
        assert_eq!(
            CoreCommand::App(AppCommand::MarkActivityRead {
                request_id: mark_request_id,
                target: ActivityMarkReadTarget::All,
            })
            .request_id(),
            mark_request_id
        );

        for debug in [
            format!(
                "{:?}",
                AppCommand::PaginateActivity {
                    request_id: fake_rid(24),
                    tab: ActivityTab::Unread,
                    cursor: Some("private-page-token".to_owned()),
                }
            ),
            format!("{mark:?}"),
        ] {
            assert!(!debug.contains("private-page-token"), "{debug}");
            assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
            assert!(!debug.contains("$private-event:example.invalid"), "{debug}");
        }
    }

    #[test]
    fn upload_staging_commands_require_ready_session_and_redact_debug() {
        use koushi_state::{StagedUploadKind, build_formatted_message_draft};

        let set_request_id = fake_rid(24);
        let update_caption_request_id = fake_rid(25);
        let update_compression_request_id = fake_rid(26);
        let clear_request_id = fake_rid(27);
        let target = koushi_state::ComposerTarget::Main {
            room_id: "!private-room:example.invalid".to_owned(),
        };
        let set = AppCommand::SetUploadStaging {
            request_id: set_request_id,
            target: target.clone(),
            items: vec![StagedUploadItem {
                staged_id: "private-staged-id".to_owned(),
                room_id: "!private-room:example.invalid".to_owned(),
                position: 1,
                filename: "private-image.png".to_owned(),
                mime_type: "image/png".to_owned(),
                byte_count: 99,
                kind: StagedUploadKind::Image {
                    width: Some(4),
                    height: Some(2),
                },
                caption: Some(build_formatted_message_draft(
                    "private staged caption",
                    MentionIntent::default(),
                )),
                compression_choice: StagedUploadCompressionChoice::Original,
                preparation: Default::default(),
            }],
        };
        let update_caption = AppCommand::UpdateStagedUploadCaption {
            request_id: update_caption_request_id,
            target: target.clone(),
            staged_id: "private-staged-id".to_owned(),
            caption: Some(build_formatted_message_draft(
                "private staged caption",
                MentionIntent::default(),
            )),
        };
        let update_compression = AppCommand::UpdateStagedUploadCompression {
            request_id: update_compression_request_id,
            target: target.clone(),
            staged_id: "private-staged-id".to_owned(),
            compression_choice: StagedUploadCompressionChoice::Compressed {
                mode: ImageUploadCompressionMode::Always,
            },
        };
        let clear = AppCommand::ClearUploadStaging {
            request_id: clear_request_id,
            target: target.clone(),
        };

        assert_eq!(CoreCommand::App(set).request_id(), set_request_id);
        for command in [
            AppCommand::SetUploadStaging {
                request_id: set_request_id,
                target: target.clone(),
                items: Vec::new(),
            },
            AppCommand::UpdateStagedUploadCaption {
                request_id: update_caption_request_id,
                target: target.clone(),
                staged_id: "private-staged-id".to_owned(),
                caption: None,
            },
            AppCommand::UpdateStagedUploadCompression {
                request_id: update_compression_request_id,
                target: target.clone(),
                staged_id: "private-staged-id".to_owned(),
                compression_choice: StagedUploadCompressionChoice::Original,
            },
            AppCommand::ClearUploadStaging {
                request_id: clear_request_id,
                target: target.clone(),
            },
        ] {
            assert!(CoreCommand::App(command).requires_ready_session());
        }

        for debug in [
            format!("{update_caption:?}"),
            format!("{update_compression:?}"),
            format!("{clear:?}"),
            format!(
                "{:?}",
                AppCommand::SetUploadStaging {
                    request_id: set_request_id,
                    target,
                    items: vec![StagedUploadItem {
                        staged_id: "private-staged-id".to_owned(),
                        room_id: "!private-room:example.invalid".to_owned(),
                        position: 1,
                        filename: "private-image.png".to_owned(),
                        mime_type: "image/png".to_owned(),
                        byte_count: 99,
                        kind: StagedUploadKind::File,
                        caption: None,
                        compression_choice: StagedUploadCompressionChoice::NotApplicable,
                        preparation: Default::default(),
                    }],
                }
            ),
        ] {
            assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
            assert!(!debug.contains("private-staged-id"), "{debug}");
            assert!(!debug.contains("private-image.png"), "{debug}");
            assert!(!debug.contains("private staged caption"), "{debug}");
        }
    }

    #[test]
    fn open_timeline_at_timestamp_requires_ready_session_and_redacts_debug() {
        let request_id = fake_rid(28);
        let command = AppCommand::OpenTimelineAtTimestamp {
            request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            timestamp_ms: 1_718_000_000_000,
        };

        assert_eq!(CoreCommand::App(command).request_id(), request_id);
        assert!(
            CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
                request_id,
                room_id: "!private-room:example.invalid".to_owned(),
                timestamp_ms: 1_718_000_000_000,
            })
            .requires_ready_session()
        );
        let debug = format!(
            "{:?}",
            AppCommand::OpenTimelineAtTimestamp {
                request_id,
                room_id: "!private-room:example.invalid".to_owned(),
                timestamp_ms: 1_718_000_000_000,
            }
        );
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(debug.contains("Timestamp(..)"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
        assert!(!debug.contains("1718000000000"), "{debug}");
    }

    #[test]
    fn focused_projection_commands_redact_matrix_identifiers() {
        let request_id = fake_rid(29);
        let key = TimelineKey {
            account_key: AccountKey("@private:example.invalid".to_owned()),
            kind: crate::ids::TimelineKind::Focused {
                room_id: "!private-room:example.invalid".to_owned(),
                event_id: "$private-event:example.invalid".to_owned(),
            },
        };
        let debug = format!(
            "{:?} {:?}",
            AppCommand::OpenAnchoredTimeline {
                request_id,
                room_id: "!private-room:example.invalid".to_owned(),
                event_id: "$private-event:example.invalid".to_owned(),
            },
            AppCommand::AcknowledgeTimelineProjection {
                request_id,
                projection_request_id: fake_rid(28),
                key,
                generation: TimelineGeneration(3),
            }
        );
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(debug.contains("TimelineKey(..)"), "{debug}");
        for private in [
            "@private:example.invalid",
            "!private-room:example.invalid",
            "$private-event:example.invalid",
        ] {
            assert!(!debug.contains(private), "{debug}");
        }
    }

    #[test]
    fn acknowledge_timeline_batch_rendered_preserves_fences_and_redacts_key() {
        let request_id = fake_rid(30);
        let command = AppCommand::AcknowledgeTimelineBatchRendered {
            request_id,
            key: TimelineKey {
                account_key: AccountKey("@private:example.invalid".to_owned()),
                kind: crate::ids::TimelineKind::Room {
                    room_id: "!private-room:example.invalid".to_owned(),
                },
            },
            actor_generation: 9,
            timeline_generation: TimelineGeneration(3),
            repair_generation: 11,
            batch_id: crate::ids::TimelineBatchId(5),
        };

        assert_eq!(CoreCommand::App(command).request_id(), request_id);
        let debug = format!(
            "{:?}",
            AppCommand::AcknowledgeTimelineBatchRendered {
                request_id,
                key: TimelineKey {
                    account_key: AccountKey("@private:example.invalid".to_owned()),
                    kind: crate::ids::TimelineKind::Room {
                        room_id: "!private-room:example.invalid".to_owned(),
                    },
                },
                actor_generation: 9,
                timeline_generation: TimelineGeneration(3),
                repair_generation: 11,
                batch_id: crate::ids::TimelineBatchId(5),
            }
        );
        for expected in [
            "actor_generation: 9",
            "repair_generation: 11",
            "TimelineBatchId(5)",
        ] {
            assert!(debug.contains(expected), "{debug}");
        }
        assert!(debug.contains("TimelineKey(..)"), "{debug}");
        assert!(!debug.contains("@private:example.invalid"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    }

    #[test]
    fn native_attention_command_debug_redacts_candidate_labels() {
        let command = AppCommand::UpdateNativeAttentionState {
            request_id: fake_rid(27),
            attention: NativeAttentionState {
                summary: NativeAttentionSummary {
                    unread_count: 4,
                    highlight_count: 1,
                    badge_count: 4,
                    candidate: Some(NativeAttentionCandidate {
                        room_display_name: "Private Room Name".to_owned(),
                        kind: RoomAttentionKind::Mention,
                        unread_count: 4,
                        highlight_count: 1,
                    }),
                    capabilities: NativeAttentionCapabilities {
                        notifications: NativeAttentionCapability::Available,
                        badge: NativeAttentionCapability::Available,
                        overlay_icon: NativeAttentionCapability::Unknown,
                        sound: NativeAttentionCapability::Unknown,
                        tray: NativeAttentionCapability::Unavailable,
                        activation: NativeAttentionCapability::Unknown,
                    },
                },
                dispatch: NativeAttentionDispatchState::Suppressed {
                    reason: NativeAttentionSuppressionReason::WindowFocused,
                },
            },
        };

        let debug = format!("{command:?}");

        assert!(debug.contains("UpdateNativeAttentionState"), "{debug}");
        assert!(debug.contains("unread_count"), "{debug}");
        assert!(debug.contains("suppressed"), "{debug}");
        assert!(!debug.contains("Private Room Name"), "{debug}");
    }

    #[test]
    fn profile_command_debug_redacts_display_name_and_avatar_bytes() {
        let display_name = AccountCommand::SetDisplayName {
            request_id: fake_rid(9),
            display_name: Some("Private Display".to_owned()),
        };
        let alias = AccountCommand::SetLocalUserAlias {
            request_id: fake_rid(11),
            user_id: "@private:example.invalid".to_owned(),
            alias: Some("Private Alias".to_owned()),
        };
        let avatar = AccountCommand::SetAvatar {
            request_id: fake_rid(10),
            request: SetAvatarRequest {
                mime_type: "image/png".to_owned(),
                bytes: vec![9, 8, 7, 6],
            },
        };

        let display_debug = format!("{display_name:?}");
        assert!(display_debug.contains("SetDisplayName"), "{display_debug}");
        assert!(
            !display_debug.contains("Private Display"),
            "{display_debug}"
        );

        let alias_debug = format!("{alias:?}");
        assert!(alias_debug.contains("SetLocalUserAlias"), "{alias_debug}");
        assert!(
            !alias_debug.contains("@private:example.invalid"),
            "{alias_debug}"
        );
        assert!(!alias_debug.contains("Private Alias"), "{alias_debug}");

        let avatar_debug = format!("{avatar:?}");
        assert!(avatar_debug.contains("SetAvatar"), "{avatar_debug}");
        assert!(avatar_debug.contains("image/png"), "{avatar_debug}");
        assert!(avatar_debug.contains("bytes_len"), "{avatar_debug}");
        assert!(!avatar_debug.contains("9, 8, 7, 6"), "{avatar_debug}");
    }
}
