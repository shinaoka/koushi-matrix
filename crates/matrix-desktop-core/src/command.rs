//! Public command boundary. Every command carries a runtime-scoped
//! `RequestId`. Secret-bearing payloads redact `Debug`.

use std::fmt;

use matrix_desktop_state::{
    ActivityMarkReadTarget, ActivityTab, DirectoryQuery, IdentityResetAuthRequest,
    JapaneseCatalogProfile, LocalEncryptionHealth, LoginRequest, MentionIntent,
    NativeAttentionSummary, PresenceKind, RecoveryRequest, RoomModerationAction, RoomSettingChange,
    RoomTagKind, SettingsPatch, VerificationCancelReason, VerificationTarget,
};

use crate::ids::{AccountKey, RequestId, TimelineKey};

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
                | AppCommand::SetThreadComposerDraft { request_id, .. }
                | AppCommand::OpenThread { request_id, .. }
                | AppCommand::CloseThread { request_id }
                | AppCommand::OpenFocusedContext { request_id, .. }
                | AppCommand::CloseFocusedContext { request_id }
                | AppCommand::UpdateSettings { request_id, .. }
                | AppCommand::OpenActivity { request_id }
                | AppCommand::CloseActivity { request_id }
                | AppCommand::SetActivityTab { request_id, .. }
                | AppCommand::PaginateActivity { request_id, .. }
                | AppCommand::MarkActivityRead { request_id, .. }
                | AppCommand::RecordLocalEncryptionHealth { request_id, .. }
                | AppCommand::UpdateNativeAttentionSummary { request_id, .. }
                | AppCommand::UpdateJapaneseCatalogProfile { request_id, .. },
            ) => *request_id,
            Self::Account(command) => match command {
                AccountCommand::LoginPassword { request_id, .. }
                | AccountCommand::RestoreSession { request_id, .. }
                | AccountCommand::RestoreLastSession { request_id }
                | AccountCommand::QuerySavedSessions { request_id }
                | AccountCommand::SubmitRecovery { request_id, .. }
                | AccountCommand::RequestVerification { request_id, .. }
                | AccountCommand::AcceptVerification { request_id, .. }
                | AccountCommand::ConfirmSasVerification { request_id, .. }
                | AccountCommand::CancelVerification { request_id, .. }
                | AccountCommand::BootstrapCrossSigning { request_id, .. }
                | AccountCommand::EnableKeyBackup { request_id, .. }
                | AccountCommand::RestoreKeyBackup { request_id, .. }
                | AccountCommand::ResetIdentity { request_id }
                | AccountCommand::SubmitIdentityResetAuth { request_id, .. }
                | AccountCommand::SetPresence { request_id, .. }
                | AccountCommand::SetDisplayName { request_id, .. }
                | AccountCommand::SetAvatar { request_id, .. }
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
                | RoomCommand::UpdateRoomSetting { request_id, .. }
                | RoomCommand::ModerateRoomMember { request_id, .. }
                | RoomCommand::SelectSpace { request_id, .. }
                | RoomCommand::SelectRoom { request_id, .. } => *request_id,
            },
            Self::Timeline(command) => match command {
                TimelineCommand::Subscribe { request_id, .. }
                | TimelineCommand::Unsubscribe { request_id, .. }
                | TimelineCommand::Paginate { request_id, .. }
                | TimelineCommand::SendText { request_id, .. }
                | TimelineCommand::SendReply { request_id, .. }
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
                | TimelineCommand::ToggleReaction { request_id, .. } => *request_id,
            },
            Self::Search(command) => match command {
                SearchCommand::Query { request_id, .. } => *request_id,
            },
        }
    }

    /// Commands that require a `Ready` session before they are routed.
    pub fn requires_ready_session(&self) -> bool {
        matches!(
            self,
            Self::Sync(_) | Self::Room(_) | Self::Timeline(_) | Self::Search(_)
        ) || matches!(
            self,
            Self::Account(command) if command.requires_ready_session()
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
    SetThreadComposerDraft {
        request_id: RequestId,
        room_id: String,
        root_event_id: String,
        draft: String,
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
    CloseFocusedContext {
        request_id: RequestId,
    },
    UpdateSettings {
        request_id: RequestId,
        patch: SettingsPatch,
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
    MarkActivityRead {
        request_id: RequestId,
        target: ActivityMarkReadTarget,
    },
    RecordLocalEncryptionHealth {
        request_id: RequestId,
        health: LocalEncryptionHealth,
    },
    UpdateNativeAttentionSummary {
        request_id: RequestId,
        summary: NativeAttentionSummary,
    },
    UpdateJapaneseCatalogProfile {
        request_id: RequestId,
        profile: JapaneseCatalogProfile,
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
            Self::CloseFocusedContext { request_id } => formatter
                .debug_struct("CloseFocusedContext")
                .field("request_id", request_id)
                .finish(),
            Self::UpdateSettings { request_id, patch } => formatter
                .debug_struct("UpdateSettings")
                .field("request_id", request_id)
                .field("patch_fields", &settings_patch_field_names(patch))
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
            Self::MarkActivityRead { request_id, target } => formatter
                .debug_struct("MarkActivityRead")
                .field("request_id", request_id)
                .field("target", target)
                .finish(),
            Self::RecordLocalEncryptionHealth { request_id, health } => formatter
                .debug_struct("RecordLocalEncryptionHealth")
                .field("request_id", request_id)
                .field("health", health)
                .finish(),
            Self::UpdateNativeAttentionSummary {
                request_id,
                summary,
            } => formatter
                .debug_struct("UpdateNativeAttentionSummary")
                .field("request_id", request_id)
                .field("unread_count", &summary.unread_count)
                .field("highlight_count", &summary.highlight_count)
                .field("badge_count", &summary.badge_count)
                .field(
                    "candidate",
                    &summary.candidate.as_ref().map(|_| "AttentionCandidate(..)"),
                )
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
    fields
}

// LoginRequest and RecoveryRequest redact their own Debug in
// matrix-desktop-state (username, password, device name, recovery secret).
pub enum AccountCommand {
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
    SubmitRecovery {
        request_id: RequestId,
        request: RecoveryRequest,
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
        auth: Option<matrix_desktop_state::AuthSecret>,
    },
    EnableKeyBackup {
        request_id: RequestId,
        passphrase: Option<matrix_desktop_state::AuthSecret>,
    },
    RestoreKeyBackup {
        request_id: RequestId,
        version: Option<String>,
        request: RecoveryRequest,
    },
    ResetIdentity {
        request_id: RequestId,
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
    SetAvatar {
        request_id: RequestId,
        request: SetAvatarRequest,
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
        matches!(
            self,
            Self::RequestVerification { .. }
                | Self::AcceptVerification { .. }
                | Self::ConfirmSasVerification { .. }
                | Self::CancelVerification { .. }
                | Self::BootstrapCrossSigning { .. }
                | Self::EnableKeyBackup { .. }
                | Self::ResetIdentity { .. }
                | Self::SubmitIdentityResetAuth { .. }
                | Self::SetPresence { .. }
                | Self::SetDisplayName { .. }
                | Self::SetAvatar { .. }
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
            Self::SubmitRecovery {
                request_id,
                request,
            } => formatter
                .debug_struct("SubmitRecovery")
                .field("request_id", request_id)
                .field("request", request)
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
            Self::ResetIdentity { request_id } => formatter
                .debug_struct("ResetIdentity")
                .field("request_id", request_id)
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

pub enum RoomCommand {
    CreateRoom {
        request_id: RequestId,
        name: String,
        encrypted: bool,
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
    SelectSpace {
        request_id: RequestId,
        space_id: Option<String>,
    },
    SelectRoom {
        request_id: RequestId,
        room_id: String,
    },
}

impl fmt::Debug for RoomCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateRoom {
                request_id,
                encrypted,
                ..
            } => formatter
                .debug_struct("CreateRoom")
                .field("request_id", request_id)
                .field("name", &"RoomName(..)")
                .field("encrypted", encrypted)
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
            Self::SelectSpace {
                request_id,
                space_id,
            } => formatter
                .debug_struct("SelectSpace")
                .field("request_id", request_id)
                .field("space_id", &space_id.as_ref().map(|_| "RoomId(..)"))
                .finish(),
            Self::SelectRoom { request_id, .. } => formatter
                .debug_struct("SelectRoom")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadMediaKind {
    Image {
        width: Option<u64>,
        height: Option<u64>,
    },
    File,
}

#[derive(Clone, Eq, PartialEq)]
pub struct UploadMediaRequest {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub kind: UploadMediaKind,
    pub caption: Option<String>,
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
    SendText {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        body: String,
        mentions: MentionIntent,
    },
    SendReply {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        in_reply_to_event_id: String,
        body: String,
        mentions: MentionIntent,
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
            } => formatter
                .debug_struct("UploadAndSendMedia")
                .field("request_id", request_id)
                .field("key", key)
                .field("transaction_id", transaction_id)
                .field("mime_type", &request.mime_type)
                .field("kind", &request.kind)
                .field("filename", &"MediaFilename(..)")
                .field("bytes", &"MediaBytes(..)")
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
        }
    }
}

pub enum SearchCommand {
    Query {
        request_id: RequestId,
        query: String,
        scope: SearchScope,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchScope {
    Global,
    Room { room_id: String },
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
        }
    }
}

#[cfg(test)]
mod tests {
    use matrix_desktop_state::{MentionIntent, MentionTarget};

    use super::*;

    fn fake_rid(seq: u64) -> RequestId {
        RequestId {
            connection_id: crate::ids::RuntimeConnectionId(999),
            sequence: seq,
        }
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
    fn upload_media_debug_redacts_filename_caption_and_bytes() {
        let command = TimelineCommand::UploadAndSendMedia {
            request_id: fake_rid(8),
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
                caption: Some("private caption".to_owned()),
            },
        };

        let debug = format!("{command:?}");
        assert!(debug.contains("UploadAndSendMedia"), "{debug}");
        assert!(debug.contains("txn-media"), "{debug}");
        assert!(debug.contains("image/png"), "{debug}");
        assert!(!debug.contains("private-fixture-name.png"), "{debug}");
        assert!(!debug.contains("private caption"), "{debug}");
        assert!(!debug.contains("1, 2, 3, 4"), "{debug}");
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
    fn directory_commands_debug_redacts_query_alias_and_server() {
        let query = RoomCommand::QueryDirectory {
            request_id: fake_rid(13),
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
        use matrix_desktop_state::{RoomJoinRule, RoomModerationAction, RoomSettingChange};

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

        for debug in [
            format!(
                "{:?}",
                RoomCommand::LoadRoomSettings {
                    request_id: fake_rid(19),
                    room_id: "!private-room:example.invalid".to_owned(),
                }
            ),
            format!("{update:?}"),
            format!("{moderation:?}"),
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
        use matrix_desktop_state::{ActivityMarkReadTarget, ActivityTab};

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
    fn profile_command_debug_redacts_display_name_and_avatar_bytes() {
        let display_name = AccountCommand::SetDisplayName {
            request_id: fake_rid(9),
            display_name: Some("Private Display".to_owned()),
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

        let avatar_debug = format!("{avatar:?}");
        assert!(avatar_debug.contains("SetAvatar"), "{avatar_debug}");
        assert!(avatar_debug.contains("image/png"), "{avatar_debug}");
        assert!(avatar_debug.contains("bytes_len"), "{avatar_debug}");
        assert!(!avatar_debug.contains("9, 8, 7, 6"), "{avatar_debug}");
    }
}
