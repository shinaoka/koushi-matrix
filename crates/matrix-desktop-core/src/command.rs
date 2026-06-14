//! Public command boundary. Every command carries a runtime-scoped
//! `RequestId`. Secret-bearing payloads redact `Debug`.

use std::fmt;

use matrix_desktop_state::{
    IdentityResetAuthRequest, LoginRequest, RecoveryRequest, SettingsPatch,
    VerificationCancelReason, VerificationTarget,
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
                | AppCommand::UpdateSettings { request_id, .. },
            ) => *request_id,
            Self::Account(command) => match command {
                AccountCommand::LoginPassword { request_id, .. }
                | AccountCommand::RestoreSession { request_id, .. }
                | AccountCommand::RestoreLastSession { request_id }
                | AccountCommand::QuerySavedSessions { request_id }
                | AccountCommand::SubmitRecovery { request_id, .. }
                | AccountCommand::RequestVerification { request_id, .. }
                | AccountCommand::AcceptVerification { request_id }
                | AccountCommand::ConfirmSasVerification { request_id }
                | AccountCommand::CancelVerification { request_id, .. }
                | AccountCommand::BootstrapCrossSigning { request_id }
                | AccountCommand::EnableKeyBackup { request_id }
                | AccountCommand::RestoreKeyBackup { request_id, .. }
                | AccountCommand::ResetIdentity { request_id }
                | AccountCommand::SubmitIdentityResetAuth { request_id, .. }
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
                | RoomCommand::CreateSpace { request_id, .. }
                | RoomCommand::SetSpaceChild { request_id, .. }
                | RoomCommand::InviteUser { request_id, .. }
                | RoomCommand::JoinRoom { request_id, .. }
                | RoomCommand::LeaveRoom { request_id, .. }
                | RoomCommand::ForgetRoom { request_id, .. }
                | RoomCommand::SelectSpace { request_id, .. }
                | RoomCommand::SelectRoom { request_id, .. } => *request_id,
            },
            Self::Timeline(command) => match command {
                TimelineCommand::Subscribe { request_id, .. }
                | TimelineCommand::Unsubscribe { request_id, .. }
                | TimelineCommand::Paginate { request_id, .. }
                | TimelineCommand::SendText { request_id, .. }
                | TimelineCommand::SendReply { request_id, .. }
                | TimelineCommand::EditText { request_id, .. }
                | TimelineCommand::Redact { request_id, .. }
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
    },
    ConfirmSasVerification {
        request_id: RequestId,
    },
    CancelVerification {
        request_id: RequestId,
        reason: VerificationCancelReason,
    },
    BootstrapCrossSigning {
        request_id: RequestId,
    },
    EnableKeyBackup {
        request_id: RequestId,
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
        request: IdentityResetAuthRequest,
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
                | Self::RestoreKeyBackup { .. }
                | Self::ResetIdentity { .. }
                | Self::SubmitIdentityResetAuth { .. }
        )
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
            Self::AcceptVerification { request_id } => formatter
                .debug_struct("AcceptVerification")
                .field("request_id", request_id)
                .finish(),
            Self::ConfirmSasVerification { request_id } => formatter
                .debug_struct("ConfirmSasVerification")
                .field("request_id", request_id)
                .finish(),
            Self::CancelVerification { request_id, reason } => formatter
                .debug_struct("CancelVerification")
                .field("request_id", request_id)
                .field("reason", reason)
                .finish(),
            Self::BootstrapCrossSigning { request_id } => formatter
                .debug_struct("BootstrapCrossSigning")
                .field("request_id", request_id)
                .finish(),
            Self::EnableKeyBackup { request_id } => formatter
                .debug_struct("EnableKeyBackup")
                .field("request_id", request_id)
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
                request,
            } => formatter
                .debug_struct("SubmitIdentityResetAuth")
                .field("request_id", request_id)
                .field("request", request)
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

#[derive(Debug)]
pub enum RoomCommand {
    CreateRoom {
        request_id: RequestId,
        name: String,
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
    SelectSpace {
        request_id: RequestId,
        space_id: Option<String>,
    },
    SelectRoom {
        request_id: RequestId,
        room_id: String,
    },
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
    },
    SendReply {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        in_reply_to_event_id: String,
        body: String,
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
    use super::*;

    fn fake_rid(seq: u64) -> RequestId {
        RequestId {
            connection_id: crate::ids::RuntimeConnectionId(999),
            sequence: seq,
        }
    }

    #[test]
    fn send_reply_debug_redacts_body_and_event_ids() {
        let command = TimelineCommand::SendReply {
            request_id: fake_rid(7),
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
            transaction_id: "txn-reply".to_owned(),
            in_reply_to_event_id: "$event:test".to_owned(),
            body: "secret reply body".to_owned(),
        };

        let debug = format!("{command:?}");
        assert!(debug.contains("SendReply"), "{debug}");
        assert!(debug.contains("txn-reply"), "{debug}");
        assert!(!debug.contains("secret reply body"), "{debug}");
        assert!(!debug.contains("$event:test"), "{debug}");
    }
}
