use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{AccountKey, RequestId};
use koushi_state::{
    DirectoryQuery, InviteScopeSelection, RoomModerationAction, RoomSettingChange, RoomTagKind,
};

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
    pub invited_only: bool,
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
            .field("invited_only", &self.invited_only)
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
/// The Space a new room is created in. Core derives the routing for both
/// relationship events from the SDK (#1007); room version 12 Space IDs carry
/// no server name for a renderer to extract.
pub struct CreateRoomParentSpace {
    pub space_id: String,
}

impl fmt::Debug for CreateRoomParentSpace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateRoomParentSpace")
            .field("space_id", &"RoomId(..)")
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
    /// Start an advisory availability check of a create-room address local
    /// part on the account's server (#1006). It replaces any earlier check;
    /// the result is `AppState.room_address_availability`, never a
    /// reservation.
    CheckRoomAddressAvailability {
        request_id: RequestId,
        alias_localpart: String,
    },
    /// Cancel the advisory check (the create dialog closed or the address
    /// no longer needs one).
    ClearRoomAddressAvailability {
        request_id: RequestId,
    },
    /// Link a joined room under a joined Space. Core derives the `via`
    /// routing from the SDK (room version 12 IDs carry no server name).
    SetSpaceChild {
        request_id: RequestId,
        space_id: String,
        child_room_id: String,
    },
    InviteUser {
        request_id: RequestId,
        room_id: String,
        user_id: String,
    },
    LoadSpaceMembers {
        request_id: RequestId,
        space_id: String,
        generation: u64,
    },
    /// Issue #961: fetch every child room the Space advertises, with the
    /// account's membership in each.
    LoadSpaceChildren {
        request_id: RequestId,
        space_id: String,
        generation: u64,
    },
    InviteUserToSpace {
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
    },
    CancelSpaceInvite {
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
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
    RefreshPinnedEvents {
        request_id: RequestId,
        room_id: String,
    },
    QueryDirectory {
        request_id: RequestId,
        query: DirectoryQuery,
    },
    PreviewJoinTarget {
        request_id: RequestId,
        /// `#alias:server` or `!id:server`.
        room_id_or_alias: String,
        /// Servers to try when the homeserver does not already know the room.
        via_servers: Vec<String>,
    },
    DismissDirectoryPreview {
        request_id: RequestId,
    },
    JoinDirectoryRoom {
        request_id: RequestId,
        /// `#alias:server` or `!id:server`.
        room_id_or_alias: String,
        /// Servers to try when the homeserver does not already know the room.
        via_servers: Vec<String>,
    },
    LoadRoomSettings {
        request_id: RequestId,
        room_id: String,
    },
    QueryMentionCandidates {
        request_id: RequestId,
        account_key: AccountKey,
        room_id: String,
        surface: koushi_state::MentionSurface,
        query: String,
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
    UpdateSpaceMemberRole {
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
        expected_power_levels_revision: Option<String>,
        expected_power_level: i64,
        power_level: i64,
        confirmed: bool,
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
    /// Debug control: discard the current outbound Megolm key. The next
    /// ordinary encrypted send creates and shares the replacement.
    ForceRotateOutboundSession {
        request_id: RequestId,
        room_id: String,
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
            Self::CheckRoomAddressAvailability { request_id, .. } => formatter
                .debug_struct("CheckRoomAddressAvailability")
                .field("request_id", request_id)
                .field("alias_localpart", &"[redacted]")
                .finish(),
            Self::ClearRoomAddressAvailability { request_id } => formatter
                .debug_struct("ClearRoomAddressAvailability")
                .field("request_id", request_id)
                .finish(),
            Self::SetSpaceChild { request_id, .. } => formatter
                .debug_struct("SetSpaceChild")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("child_room_id", &"RoomId(..)")
                .finish(),
            Self::InviteUser { request_id, .. } => formatter
                .debug_struct("InviteUser")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::LoadSpaceMembers {
                request_id,
                generation,
                ..
            } => formatter
                .debug_struct("LoadSpaceMembers")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("generation", generation)
                .finish(),
            Self::LoadSpaceChildren {
                request_id,
                generation,
                ..
            } => formatter
                .debug_struct("LoadSpaceChildren")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("generation", generation)
                .finish(),
            Self::InviteUserToSpace {
                request_id,
                generation,
                ..
            } => formatter
                .debug_struct("InviteUserToSpace")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .field("generation", generation)
                .finish(),
            Self::CancelSpaceInvite {
                request_id,
                generation,
                ..
            } => formatter
                .debug_struct("CancelSpaceInvite")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .field("generation", generation)
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
            Self::RefreshPinnedEvents { request_id, .. } => formatter
                .debug_struct("RefreshPinnedEvents")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
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
            Self::PreviewJoinTarget {
                request_id,
                via_servers,
                ..
            } => formatter
                .debug_struct("PreviewJoinTarget")
                .field("request_id", request_id)
                .field("room_id_or_alias", &"RoomIdOrAlias(..)")
                .field("via_server_count", &via_servers.len())
                .finish(),
            Self::DismissDirectoryPreview { request_id } => formatter
                .debug_struct("DismissDirectoryPreview")
                .field("request_id", request_id)
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
            Self::QueryMentionCandidates {
                request_id,
                surface,
                ..
            } => formatter
                .debug_struct("QueryMentionCandidates")
                .field("request_id", request_id)
                .field("surface", surface)
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
            Self::UpdateSpaceMemberRole {
                request_id,
                generation,
                expected_power_level,
                power_level,
                confirmed,
                ..
            } => formatter
                .debug_struct("UpdateSpaceMemberRole")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .field("generation", generation)
                .field("expected_power_levels_revision", &"EventId(..)")
                .field("expected_power_level", expected_power_level)
                .field("power_level", power_level)
                .field("confirmed", confirmed)
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
            Self::ForceRotateOutboundSession { request_id, .. } => formatter
                .debug_struct("ForceRotateOutboundSession")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
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

#[cfg(test)]
mod tests;
