use crate::auth::MATRIX_ROOM_LIST_SNAPSHOT_LIMIT;
#[cfg(test)]
use crate::room_operations::get_room_settings_snapshot;
use crate::{
    MatrixClientSession, MatrixPublicRoomDirectoryRoom, MatrixRoomHistoryVisibility,
    MatrixRoomJoinRule, MatrixRoomMemberRole, MatrixRoomOperationError,
    MatrixRoomOperationFailureKind, MatrixRoomPermissionFacts, MatrixRoomSettingChange,
    MatrixRoomSettingsSnapshot, MatrixTimelineError, MatrixTimelineItem, MatrixTimelineUpdate,
    MatrixUserTrustState,
};
use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
#[cfg(test)]
use koushi_state::SessionInfo;
use koushi_state::{RoomAttentionSummary, room_attention_summary};
use matrix_sdk::{
    deserialized_responses::SyncOrStrippedState,
    room::ParentSpace,
    ruma::events::{
        StateEventType, SyncStateEvent,
        direct::DirectEventContent,
        fully_read::FullyReadEventContent,
        receipt::{ReceiptThread, ReceiptType},
        room::power_levels::{RoomPowerLevelsEventContent, UserPowerLevel},
        space::child::SpaceChildEventContent,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};
use thiserror::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomListSnapshot {
    pub spaces: Vec<MatrixRoomListSpace>,
    pub rooms: Vec<MatrixRoomListRoom>,
    pub invites: Vec<MatrixInvitePreview>,
    pub user_profiles: Vec<MatrixUserProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListSpace {
    pub space_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub child_room_ids: Vec<String>,
    pub member_user_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomListRoom {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub is_dm: bool,
    pub dm_user_ids: Vec<String>,
    pub tags: MatrixRoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub marked_unread: bool,
    pub recency_stamp: Option<u64>,
    pub conversation_activity: Option<MatrixConversationActivity>,
    pub latest_event: Option<MatrixRoomLatestEventSummary>,
    pub parent_space_ids: Vec<String>,
    pub is_encrypted: bool,
    pub joined_members: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixConversationActivitySource {
    Message,
    EncryptedMessage,
    ThreadReply,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MatrixConversationActivity {
    pub timestamp_ms: u64,
    pub source: MatrixConversationActivitySource,
}

impl fmt::Debug for MatrixConversationActivity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixConversationActivity")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomLatestEventSummary {
    pub event_id: String,
    pub sender_id: Option<String>,
    pub sender_label: Option<String>,
    pub sender_avatar_mxc_uri: Option<String>,
    pub preview: Option<String>,
    pub timestamp_ms: u64,
    pub event_type: Option<String>,
    pub relation_type: Option<String>,
    pub relation_event_id: Option<String>,
    pub content_converted: bool,
    pub is_threaded: bool,
    pub is_reply: bool,
    pub has_thread_summary: bool,
    pub has_reactions: bool,
    pub is_redacted: bool,
}

pub(super) struct SdkUnreadTrace<'a> {
    unread_messages: u64,
    unread_count: u64,
    notification_count: u64,
    highlight_count: u64,
    marked_unread: bool,
    latest_event: &'a Option<MatrixRoomLatestEventSummary>,
    fully_read_event_id: Option<&'a str>,
    private_read_receipt_event_id: Option<&'a str>,
    recency_stamp_present: bool,
    conversation_activity: Option<MatrixConversationActivity>,
}

pub(super) fn trace_sdk_unread_snapshot(trace: SdkUnreadTrace<'_>) {
    let latest_event = trace.latest_event.as_ref();
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "sdk.unread", "sdk_room_snapshot")
            .field(DiagnosticField::count(
                "unread_messages",
                trace.unread_messages,
            ))
            .field(DiagnosticField::count("unread_count", trace.unread_count))
            .field(DiagnosticField::count(
                "notification_count",
                trace.notification_count,
            ))
            .field(DiagnosticField::count(
                "highlight_count",
                trace.highlight_count,
            ))
            .field(DiagnosticField::boolean(
                "marked_unread",
                trace.marked_unread,
            ))
            .field(DiagnosticField::boolean(
                "latest_event_present",
                latest_event.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "fully_read_present",
                trace.fully_read_event_id.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "private_receipt_present",
                trace.private_read_receipt_event_id.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_content_converted",
                latest_event.is_some_and(|event| event.content_converted),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_threaded",
                latest_event.is_some_and(|event| event.is_threaded),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_reply",
                latest_event.is_some_and(|event| event.is_reply),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_thread_summary",
                latest_event.is_some_and(|event| event.has_thread_summary),
            ))
            .field(DiagnosticField::boolean(
                "latest_event_reactions",
                latest_event.is_some_and(|event| event.has_reactions),
            ))
            .field(DiagnosticField::boolean(
                "recency_stamp_present",
                trace.recency_stamp_present,
            ))
            .field(DiagnosticField::boolean(
                "conversation_activity_present",
                trace.conversation_activity.is_some(),
            ))
            .field(DiagnosticField::token(
                "conversation_activity_source",
                conversation_activity_source_token(trace.conversation_activity),
            )),
    );
}

fn conversation_activity_source_token(
    activity: Option<MatrixConversationActivity>,
) -> &'static str {
    match activity.map(|activity| activity.source) {
        Some(MatrixConversationActivitySource::Message) => "message",
        Some(MatrixConversationActivitySource::EncryptedMessage) => "encrypted_message",
        Some(MatrixConversationActivitySource::ThreadReply) => "thread_reply",
        None => "none",
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomTags {
    pub favourite: Option<MatrixRoomTagInfo>,
    pub low_priority: Option<MatrixRoomTagInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomTagInfo {
    pub order: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomTagKind {
    Favourite,
    LowPriority,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixInvitePreview {
    pub room_id: String,
    pub display_name: String,
    pub avatar_mxc_uri: Option<String>,
    pub topic: Option<String>,
    pub inviter_display_name: Option<String>,
    pub inviter_user_id: Option<String>,
    pub is_dm: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomListError {
    #[error("Matrix room list failed")]
    Sdk,
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixUserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_mxc_uri: Option<String>,
}

impl fmt::Debug for MatrixUserProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixUserProfile")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field("has_avatar", &self.avatar_mxc_uri.is_some())
            .finish()
    }
}

/// Local-only membership facts for a Space and all of its current child rooms.
///
/// This deliberately preserves the distinction between a Space `JOIN`, a
/// Space `INVITE`, and a child-room-only `JOIN`. Consumers must not infer these
/// classes from a flattened `ACTIVE` member list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixSpaceMemberRoleOption {
    pub power_level: i64,
    pub role: MatrixRoomMemberRole,
    pub requires_confirmation: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixSpaceMembersProjection {
    pub space_id: String,
    /// The child-room scope used by RoomActor for sync-driven local refresh.
    /// This is store-derived metadata, not member/profile data.
    #[serde(default)]
    pub child_room_ids: Vec<String>,
    pub space_joined: Vec<MatrixSpaceMemberEntry>,
    pub space_invited: Vec<MatrixSpaceMemberEntry>,
    pub child_room_only: Vec<MatrixSpaceMemberEntry>,
    pub child_room_profiles: Vec<MatrixSpaceMemberEntry>,
    #[serde(default)]
    pub space_joined_input_count: usize,
    #[serde(default)]
    pub space_invited_input_count: usize,
    #[serde(default)]
    pub child_join_input_count: usize,
    #[serde(default)]
    pub child_join_union_count: usize,
    #[serde(default)]
    pub duplicate_child_membership_count: usize,
    pub child_room_count: usize,
    pub complete_child_room_count: usize,
    pub incomplete_child_room_count: usize,
    #[serde(default)]
    pub power_levels_revision: Option<String>,
    #[serde(default)]
    pub can_edit_roles: bool,
}

impl fmt::Debug for MatrixSpaceMembersProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSpaceMembersProjection")
            .field("space_id", &"RoomId(..)")
            .field("space_joined_count", &self.space_joined.len())
            .field("space_invited_count", &self.space_invited.len())
            .field("child_room_only_count", &self.child_room_only.len())
            .field("child_room_profile_count", &self.child_room_profiles.len())
            .field("space_joined_input_count", &self.space_joined_input_count)
            .field("space_invited_input_count", &self.space_invited_input_count)
            .field("child_join_input_count", &self.child_join_input_count)
            .field("child_join_union_count", &self.child_join_union_count)
            .field(
                "duplicate_child_membership_count",
                &self.duplicate_child_membership_count,
            )
            .field("child_room_count", &self.child_room_count)
            .field("complete_child_room_count", &self.complete_child_room_count)
            .field(
                "incomplete_child_room_count",
                &self.incomplete_child_room_count,
            )
            .field(
                "power_levels_revision",
                &self.power_levels_revision.as_ref().map(|_| "EventId(..)"),
            )
            .field("can_edit_roles", &self.can_edit_roles)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixSpaceMemberEntry {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: MatrixRoomMemberRole,
    pub child_room_ids: Vec<String>,
    #[serde(default)]
    pub role_options: Vec<MatrixSpaceMemberRoleOption>,
}

impl fmt::Debug for MatrixSpaceMemberEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSpaceMemberEntry")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("power_level", &self.power_level)
            .field("role", &self.role)
            .field("child_room_count", &self.child_room_ids.len())
            .field("role_option_count", &self.role_options.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixRoomMemberSummary {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: MatrixRoomMemberRole,
    pub user_trust: Option<MatrixUserTrustState>,
}

impl fmt::Debug for MatrixRoomMemberSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixRoomMemberSummary")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field("has_avatar", &self.avatar_url.is_some())
            .field("power_level", &self.power_level)
            .field("role", &self.role)
            .field("user_trust", &self.user_trust)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixJoinedMemberSnapshot {
    pub members: Vec<MatrixRoomMemberSummary>,
    pub complete: bool,
    pub room_mention_allowed: Option<bool>,
}

impl fmt::Debug for MatrixJoinedMemberSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixJoinedMemberSnapshot")
            .field("member_count", &self.members.len())
            .field("complete", &self.complete)
            .field("room_mention_allowed", &self.room_mention_allowed)
            .finish()
    }
}

impl MatrixClientSession {
    pub async fn joined_member_snapshot_no_sync(
        &self,
        room_id: &str,
    ) -> Result<MatrixJoinedMemberSnapshot, MatrixRoomOperationError> {
        let room = matrix_room(self, room_id)?;
        matrix_joined_member_snapshot(&room, false).await
    }

    pub async fn refresh_joined_member_snapshot(
        &self,
        room_id: &str,
    ) -> Result<MatrixJoinedMemberSnapshot, MatrixRoomOperationError> {
        let room = matrix_room(self, room_id)?;
        matrix_joined_member_snapshot(&room, true).await
    }

    /// Read the requested member profiles from the already populated local
    /// encrypted SDK room store. This deliberately uses `get_member_no_sync`:
    /// a receipt/Seen render must never fan out to the homeserver.
    pub async fn room_member_profiles_no_sync(
        &self,
        room_id: &str,
        user_ids: &[String],
    ) -> Result<Vec<MatrixUserProfile>, MatrixRoomOperationError> {
        let room = matrix_room(self, room_id)?;
        let mut unique_user_ids = BTreeSet::new();
        for user_id in user_ids {
            if matrix_sdk::ruma::UserId::parse(user_id).is_ok() {
                unique_user_ids.insert(user_id.as_str());
            }
        }

        let mut profiles = Vec::with_capacity(unique_user_ids.len());
        for user_id in unique_user_ids {
            let Ok(user_id) = matrix_sdk::ruma::UserId::parse(user_id) else {
                continue;
            };
            let Ok(Some(member)) = room.get_member_no_sync(&user_id).await else {
                continue;
            };
            profiles.push(MatrixUserProfile {
                user_id: member.user_id().to_string(),
                display_name: member.display_name().map(ToOwned::to_owned),
                avatar_mxc_uri: member.avatar_url().map(ToString::to_string),
            });
        }
        Ok(profiles)
    }
}

pub fn room_list_snapshot_blocking(
    session: &MatrixClientSession,
) -> Result<MatrixRoomListSnapshot, MatrixRoomListError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixRoomListError::Sdk)?;

    runtime.block_on(room_list_snapshot(session))
}

#[cfg(test)]
mod joined_member_snapshot_tests {
    use matrix_sdk::{
        ruma::events::room::member::MembershipState, test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    use super::{MatrixClientSession, SessionInfo};

    async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        let info = SessionInfo {
            homeserver: server.server().uri(),
            user_id: client
                .user_id()
                .expect("mock client has a user id")
                .to_string(),
            device_id: client
                .device_id()
                .expect("mock client has a device id")
                .to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        MatrixClientSession { client, info }
    }

    #[tokio::test]
    async fn joined_member_snapshot_no_sync_is_fail_closed_and_does_not_request_members() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let room_id = matrix_sdk::ruma::room_id!("!mention-room:example.org");
        let joined = matrix_sdk::ruma::user_id!("@joined:example.org");
        let invited = matrix_sdk::ruma::user_id!("@invited:example.org");
        let left = matrix_sdk::ruma::user_id!("@left:example.org");

        server
            .mock_sync()
            .ok_and_run(&session.client(), |builder| {
                builder.add_joined_room(
                    JoinedRoomBuilder::new(room_id)
                        .add_state_event(
                            EventFactory::new()
                                .room(room_id)
                                .member(joined)
                                .display_name("Joined Member")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(room_id)
                                .member(invited)
                                .membership(MembershipState::Invite)
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(room_id)
                                .member(left)
                                .membership(MembershipState::Leave)
                                .into_raw_sync_state(),
                        ),
                );
            })
            .await;

        // No /members mock is mounted. A network request would fail this call.
        let snapshot = session
            .joined_member_snapshot_no_sync(room_id.as_str())
            .await
            .expect("cached joined member snapshot");
        assert!(!snapshot.complete);
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.members[0].user_id, joined.as_str());
        assert_eq!(
            snapshot.members[0].display_name.as_deref(),
            Some("Joined Member")
        );
    }

    #[tokio::test]
    async fn refresh_joined_member_snapshot_fetches_once_and_marks_the_snapshot_complete() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let room_id = matrix_sdk::ruma::room_id!("!mention-refresh:example.org");
        let joined = matrix_sdk::ruma::user_id!("@refreshed:example.org");

        server
            .mock_sync()
            .ok_and_run(&session.client(), |builder| {
                builder.add_joined_room(JoinedRoomBuilder::new(room_id));
            })
            .await;
        server
            .mock_get_members()
            .ok(vec![
                EventFactory::new()
                    .room(room_id)
                    .member(joined)
                    .display_name("Refreshed Member")
                    .into_raw(),
            ])
            .mock_once()
            .mount()
            .await;

        let refreshed = session
            .refresh_joined_member_snapshot(room_id.as_str())
            .await
            .expect("member refresh");
        assert!(refreshed.complete);
        assert_eq!(refreshed.members.len(), 1);
        assert_eq!(refreshed.members[0].user_id, joined.as_str());

        let cached = session
            .refresh_joined_member_snapshot(room_id.as_str())
            .await
            .expect("already complete refresh should use the cache");
        assert!(cached.complete);
        assert_eq!(cached.members, refreshed.members);
    }
}

#[cfg(test)]
mod room_permission_tests {
    use matrix_sdk::{ruma::RoomVersionId, test_utils::mocks::MatrixMockServer};
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
    use serde_json::json;

    use super::{MatrixClientSession, SessionInfo, get_room_settings_snapshot};

    async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        let info = SessionInfo {
            homeserver: server.server().uri(),
            user_id: client
                .user_id()
                .expect("mock client has a user id")
                .to_string(),
            device_id: client
                .device_id()
                .expect("mock client has a device id")
                .to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        MatrixClientSession { client, info }
    }

    #[tokio::test]
    async fn room_settings_snapshot_uses_invite_power_level_not_role_editability() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let room_id = matrix_sdk::ruma::room_id!("!permission-room:example.org");
        let client = session.client();
        let own_user_id = client.user_id().expect("mock user");
        let power_level_content: matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent = serde_json::from_value(json!({
            "ban": 50,
            "events": {},
            "events_default": 50,
            "invite": 150,
            "kick": 50,
            "redact": 50,
            "state_default": 50,
            "users": {},
            "users_default": 100
        }))
        .expect("power levels event");
        let power_levels = EventFactory::new()
            .room(room_id)
            .sender(own_user_id)
            .event(power_level_content)
            .state_key("")
            .into_raw_sync_state();

        server
            .mock_sync()
            .ok_and_run(&client, |builder| {
                builder.add_joined_room(
                    JoinedRoomBuilder::new(room_id)
                        .add_state_event(
                            EventFactory::new()
                                .room(room_id)
                                .create(own_user_id, RoomVersionId::V1)
                                .into_raw_sync_state(),
                        )
                        .add_state_event(power_levels),
                );
            })
            .await;

        let room = client.get_room(&room_id).expect("joined permission room");
        let power_levels = room.power_levels_or_default().await;
        assert_eq!(power_levels.invite, matrix_sdk::ruma::Int::from(150));
        assert_eq!(power_levels.users_default, matrix_sdk::ruma::Int::from(100));

        let snapshot = get_room_settings_snapshot(&session, room_id.as_str())
            .await
            .expect("room settings snapshot");

        assert!(snapshot.permissions.can_edit_roles);
        assert!(!snapshot.permissions.can_invite);
    }
}

#[cfg(test)]
mod space_member_projection_tests {
    use matrix_sdk::{
        ruma::{RoomVersionId, events::room::member::MembershipState},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    use super::{MatrixClientSession, SessionInfo};

    async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        let info = SessionInfo {
            homeserver: server.server().uri(),
            user_id: client
                .user_id()
                .expect("mock client has a user id")
                .to_string(),
            device_id: client
                .device_id()
                .expect("mock client has a device id")
                .to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        MatrixClientSession { client, info }
    }

    #[tokio::test]
    async fn projection_uses_local_join_and_invite_filters_and_unions_child_joins() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let client = session.client();
        let own_user = client.user_id().expect("mock user");
        let space_id = matrix_sdk::ruma::room_id!("!space-facts:example.org");
        let child_a = matrix_sdk::ruma::room_id!("!child-a:example.org");
        let child_b = matrix_sdk::ruma::room_id!("!child-b:example.org");
        let joined = matrix_sdk::ruma::user_id!("@joined:example.org");
        let invited = matrix_sdk::ruma::user_id!("@invited:example.org");
        let both = matrix_sdk::ruma::user_id!("@both:example.org");
        let child_only = matrix_sdk::ruma::user_id!("@child-only:example.org");
        let second_only = matrix_sdk::ruma::user_id!("@second-only:example.org");
        let left = matrix_sdk::ruma::user_id!("@left:example.org");
        let banned = matrix_sdk::ruma::user_id!("@banned:example.org");
        let knocked = matrix_sdk::ruma::user_id!("@knocked:example.org");

        server
            .mock_sync()
            .ok_and_run(&session.client(), |builder| {
                builder.add_joined_room(
                    JoinedRoomBuilder::new(space_id)
                        .add_state_event(
                            EventFactory::new()
                                .room(space_id)
                                .create(own_user, RoomVersionId::V1)
                                .with_space_type()
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .sender(own_user)
                                .space_child(space_id.to_owned(), child_a.to_owned())
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .sender(own_user)
                                .space_child(space_id.to_owned(), child_b.to_owned())
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(space_id)
                                .member(joined)
                                .display_name("Joined")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(space_id)
                                .member(invited)
                                .membership(MembershipState::Invite)
                                .display_name("Invited")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(space_id)
                                .member(both)
                                .display_name("Both")
                                .into_raw_sync_state(),
                        ),
                );
                builder.add_joined_room(
                    JoinedRoomBuilder::new(child_a)
                        .add_state_event(
                            EventFactory::new()
                                .room(child_a)
                                .member(child_only)
                                .display_name("Child only")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(child_a)
                                .member(both)
                                .display_name("Both in child")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(child_a)
                                .member(left)
                                .membership(MembershipState::Leave)
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(child_a)
                                .member(banned)
                                .membership(MembershipState::Ban)
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(child_a)
                                .member(knocked)
                                .membership(MembershipState::Knock)
                                .into_raw_sync_state(),
                        ),
                );
                builder.add_joined_room(
                    JoinedRoomBuilder::new(child_b)
                        .add_state_event(
                            EventFactory::new()
                                .room(child_b)
                                .member(child_only)
                                .display_name("Child only again")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(child_b)
                                .member(second_only)
                                .display_name("Second only")
                                .into_raw_sync_state(),
                        )
                        .add_state_event(
                            EventFactory::new()
                                .room(child_b)
                                .member(invited)
                                .membership(MembershipState::Invite)
                                .into_raw_sync_state(),
                        ),
                );
            })
            .await;

        let projection = super::matrix_space_members_projection(&session, space_id.as_str())
            .await
            .expect("local Space projection");

        let ids = |entries: &[super::MatrixSpaceMemberEntry]| {
            entries
                .iter()
                .map(|entry| entry.user_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&projection.space_joined),
            vec![both.to_string(), joined.to_string()]
        );
        assert_eq!(ids(&projection.space_invited), vec![invited.to_string()]);
        assert_eq!(
            ids(&projection.child_room_only),
            vec![child_only.to_string(), second_only.to_string()]
        );
        assert!(
            projection
                .child_room_profiles
                .iter()
                .any(|entry| entry.user_id == both.to_string())
        );
        assert_eq!(
            projection.child_room_only[0].child_room_ids,
            vec![child_a.to_string(), child_b.to_string()]
        );
        assert_eq!(
            projection.child_room_ids,
            vec![child_a.to_string(), child_b.to_string()]
        );
        assert_eq!(projection.child_room_count, 2);
        assert_eq!(projection.incomplete_child_room_count, 2);
        assert_eq!(projection.space_joined_input_count, 2);
        assert_eq!(projection.space_invited_input_count, 1);
        assert_eq!(projection.child_join_input_count, 4);
        assert_eq!(projection.child_join_union_count, 3);
        assert_eq!(projection.duplicate_child_membership_count, 1);
    }

    #[test]
    fn space_member_projection_debug_redacts_identifiers_and_profiles() {
        let entry = super::MatrixSpaceMemberEntry {
            user_id: "@private:example.invalid".to_owned(),
            display_name: Some("Private name".to_owned()),
            avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
            power_level: Some(100),
            role: super::MatrixRoomMemberRole::Administrator,
            child_room_ids: vec!["!child:example.invalid".to_owned()],
            role_options: Vec::new(),
        };
        let projection = super::MatrixSpaceMembersProjection {
            space_id: "!space:example.invalid".to_owned(),
            child_room_ids: vec!["!child:example.invalid".to_owned()],
            space_joined: vec![entry.clone()],
            space_invited: Vec::new(),
            child_room_only: Vec::new(),
            child_room_profiles: vec![entry.clone()],
            space_joined_input_count: 1,
            space_invited_input_count: 0,
            child_join_input_count: 0,
            child_join_union_count: 0,
            duplicate_child_membership_count: 0,
            child_room_count: 1,
            complete_child_room_count: 1,
            incomplete_child_room_count: 0,
            power_levels_revision: None,
            can_edit_roles: false,
        };

        for debug in [format!("{entry:?}"), format!("{projection:?}")] {
            assert!(debug.contains("space_joined_count") || debug.contains("child_room_count"));
            assert!(!debug.contains("@private:example.invalid"));
            assert!(!debug.contains("Private name"));
            assert!(!debug.contains("mxc://example.invalid/avatar"));
            assert!(!debug.contains("!child:example.invalid"));
        }
    }

    #[test]
    fn local_member_profile_debug_redacts_identifiers_names_and_mxc_uris() {
        let summary = super::MatrixRoomMemberSummary {
            user_id: "@private:example.invalid".to_owned(),
            display_name: Some("Private member".to_owned()),
            avatar_url: Some("mxc://example.invalid/member-avatar".to_owned()),
            power_level: Some(50),
            role: super::MatrixRoomMemberRole::Moderator,
            user_trust: Some(super::MatrixUserTrustState::Verified),
        };
        let snapshot = super::MatrixJoinedMemberSnapshot {
            members: vec![summary],
            complete: true,
            room_mention_allowed: Some(true),
        };
        let profile = super::MatrixUserProfile {
            user_id: "@private:example.invalid".to_owned(),
            display_name: Some("Private member".to_owned()),
            avatar_mxc_uri: Some("mxc://example.invalid/member-avatar".to_owned()),
        };

        for debug in [format!("{snapshot:?}"), format!("{profile:?}")] {
            assert!(!debug.contains("@private:example.invalid"), "{debug}");
            assert!(!debug.contains("Private member"), "{debug}");
            assert!(
                !debug.contains("mxc://example.invalid/member-avatar"),
                "{debug}"
            );
        }
    }
}

#[cfg(test)]
mod space_member_role_option_matrix_tests {
    use super::{MatrixRoomMemberRole, role_options_for_powers};
    use matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;

    fn levels(
        caller: UserPowerLevel,
        target: UserPowerLevel,
        target_is_self: bool,
        can_edit_roles: bool,
    ) -> Vec<i64> {
        role_options_for_powers(caller, target, target_is_self, can_edit_roles)
            .into_iter()
            .map(|option| option.power_level)
            .collect()
    }

    #[test]
    fn direct_space_role_options_cover_finite_and_infinite_power_matrix() {
        let int = |value: i32| UserPowerLevel::Int(value.into());
        assert_eq!(levels(int(100), int(0), false, true), vec![50]);
        assert_eq!(levels(int(50), int(0), false, true), Vec::<i64>::new());
        assert_eq!(levels(int(100), int(50), false, true), vec![0]);
        assert_eq!(
            levels(UserPowerLevel::Infinite, int(0), false, true),
            vec![50, 100]
        );
        assert_eq!(
            levels(int(100), UserPowerLevel::Infinite, false, true),
            Vec::<i64>::new()
        );
        assert_eq!(
            levels(UserPowerLevel::Infinite, int(100), false, true),
            vec![0, 50]
        );
        assert_eq!(levels(int(100), int(100), false, true), Vec::<i64>::new());
    }

    #[test]
    fn direct_space_role_options_reject_self_and_non_editable_callers() {
        let int = |value: i32| UserPowerLevel::Int(value.into());
        assert_eq!(levels(int(100), int(0), true, true), Vec::<i64>::new());
        assert_eq!(levels(int(100), int(0), false, false), Vec::<i64>::new());
        let options = role_options_for_powers(int(150), int(100), false, true);
        assert!(
            options
                .iter()
                .all(|option| option.role != MatrixRoomMemberRole::Creator)
        );
        assert!(options.iter().all(|option| option.power_level < 150));
    }
}

pub type MatrixDirectTargetsByRoom = BTreeMap<String, Vec<String>>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum MatrixCachedDirectAccountData {
    Present(MatrixDirectTargetsByRoom),
    #[default]
    Missing,
    StoreError,
    Invalid,
}

pub async fn cached_direct_account_data_targets_by_room(
    session: &MatrixClientSession,
) -> MatrixCachedDirectAccountData {
    match session
        .client()
        .account()
        .account_data::<DirectEventContent>()
        .await
    {
        Ok(Some(raw)) => match raw.deserialize() {
            Ok(content) => MatrixCachedDirectAccountData::Present(
                direct_account_data_targets_by_room(&content),
            ),
            Err(_) => MatrixCachedDirectAccountData::Invalid,
        },
        Ok(None) => MatrixCachedDirectAccountData::Missing,
        Err(_) => MatrixCachedDirectAccountData::StoreError,
    }
}

pub fn direct_account_data_targets_by_room(
    content: &DirectEventContent,
) -> MatrixDirectTargetsByRoom {
    let mut targets_by_room: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (user_id, room_ids) in content.iter() {
        for room_id in room_ids {
            targets_by_room
                .entry(room_id.to_string())
                .or_default()
                .insert(user_id.to_string());
        }
    }
    targets_by_room
        .into_iter()
        .map(|(room_id, targets)| (room_id, targets.into_iter().collect()))
        .collect()
}

/// Normalize joined rooms with an optional authoritative `m.direct` map.
pub async fn room_list_snapshot_from_sdk_rooms_with_direct_targets(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
    direct_targets_by_room: Option<&MatrixDirectTargetsByRoom>,
) -> MatrixRoomListSnapshot {
    matrix_room_list_snapshot_from_rooms(direct_targets_by_room, rooms).await
}

/// Normalize a room list snapshot from caller-provided SDK rooms.
///
/// This is the normalization entry point for callers that already hold the
/// room list source of truth: entries from the ONE live `RoomListService`
/// owned by the running `SyncService` (converted with
/// `RoomListItem::into_inner()`), or `client.joined_rooms()` on the
/// `LegacySync` backend. Unlike [`room_list_snapshot`], it never constructs
/// a `RoomListService` of its own.
pub async fn room_list_snapshot_from_sdk_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    room_list_snapshot_from_sdk_rooms_with_direct_targets(rooms, None).await
}

/// Normalize joined rooms from the caller's source of truth plus invited rooms
/// from the base client. The live `RoomListService` path remains the owner of
/// joined-room entries; invites are projected from `client.invited_rooms()`
/// because the live entries adapter is intentionally joined-filtered.
pub async fn room_list_snapshot_from_sdk_rooms_with_invites(
    session: &MatrixClientSession,
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    let client = session.client();
    let direct_targets_by_room = matrix_direct_account_data_targets_by_room(&client).await;
    let mut snapshot =
        matrix_room_list_snapshot_from_rooms(Some(&direct_targets_by_room), rooms).await;
    snapshot.invites = matrix_invite_previews_from_rooms(client.invited_rooms()).await;
    snapshot
}

/// One-shot room list snapshot that constructs a DISPOSABLE
/// `RoomListService` internally.
///
/// DEPRECATED FOR CORE USE (canon, overview.md RoomActor): a disposable
/// `RoomListService` is not driven by the sync loop, races the running
/// `SyncService`, and returns entries without the live service's
/// `required_state` (e.g. `m.room.create`), so space classification is
/// unreliable (deterministically broken on Conduit). The core runtime must
/// use [`room_list_snapshot_from_sdk_rooms`] with rooms taken from the live
/// service's entries or from `client.joined_rooms()`. This function remains
/// only for the legacy auth-crate QA flow, which runs without a
/// `SyncService`.
pub async fn room_list_snapshot(
    session: &MatrixClientSession,
) -> Result<MatrixRoomListSnapshot, MatrixRoomListError> {
    let client = session.client();
    let service = match matrix_sdk_ui::room_list_service::RoomListService::new_with(
        client.clone(),
        false,
        "matrix-desktop-room-list-snapshot",
        1,
    )
    .await
    {
        Ok(service) => service,
        Err(_) => {
            let direct_targets_by_room = matrix_direct_account_data_targets_by_room(&client).await;
            return Ok(matrix_room_list_snapshot_from_rooms(
                Some(&direct_targets_by_room),
                client.joined_rooms(),
            )
            .await);
        }
    };
    let all_rooms = service
        .all_rooms()
        .await
        .map_err(|_| MatrixRoomListError::Sdk)?;
    let (entries, entries_controller) =
        all_rooms.entries_with_dynamic_adapters(MATRIX_ROOM_LIST_SNAPSHOT_LIMIT);
    entries_controller.set_filter(Box::new(
        matrix_sdk_ui::room_list_service::filters::new_filter_joined(),
    ));

    let mut entries = Box::pin(entries);
    let Some(diffs) = entries.next().await else {
        return Ok(MatrixRoomListSnapshot::default());
    };

    let direct_targets_by_room = matrix_direct_account_data_targets_by_room(&client).await;
    let snapshot = matrix_room_list_snapshot_from_diffs(Some(&direct_targets_by_room), diffs).await;
    if snapshot.rooms.is_empty() && snapshot.spaces.is_empty() {
        return Ok(matrix_room_list_snapshot_from_rooms(
            Some(&direct_targets_by_room),
            client.joined_rooms(),
        )
        .await);
    }

    Ok(snapshot)
}

pub fn room_attention_summary_from_counts(
    room_display_name: Option<String>,
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_messages: u64,
    is_marked_unread: bool,
) -> Option<RoomAttentionSummary> {
    let unread_count =
        room_attention_unread_count(notification_count, unread_messages, is_marked_unread);
    let room_display_name = room_display_name.unwrap_or_else(|| "Room".to_owned());

    room_attention_summary(
        room_display_name,
        is_dm,
        notification_count,
        highlight_count,
        unread_count,
    )
}

pub(super) fn matrix_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixRoomOperationError::RoomUnavailable)
}

async fn matrix_joined_member_snapshot(
    room: &matrix_sdk::Room,
    refresh: bool,
) -> Result<MatrixJoinedMemberSnapshot, MatrixRoomOperationError> {
    let members = if refresh {
        room.members(matrix_sdk::RoomMemberships::JOIN).await
    } else {
        room.members_no_sync(matrix_sdk::RoomMemberships::JOIN)
            .await
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)?;
    let room_mention_allowed = members
        .iter()
        .find(|member| member.user_id() == room.own_user_id())
        .map(|member| member.can_trigger_room_notification());
    let mut summaries: Vec<MatrixRoomMemberSummary> = members
        .into_iter()
        .map(|member| {
            let power_level = matrix_room_member_power_level(member.power_level());
            MatrixRoomMemberSummary {
                user_id: member.user_id().to_string(),
                display_name: member.display_name().map(ToOwned::to_owned),
                avatar_url: member.avatar_url().map(ToString::to_string),
                power_level,
                role: matrix_room_member_role(power_level),
                user_trust: None,
            }
        })
        .collect();
    summaries.sort_by(|left, right| left.user_id.cmp(&right.user_id));
    Ok(MatrixJoinedMemberSnapshot {
        members: summaries,
        complete: room.are_members_synced(),
        room_mention_allowed,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SpaceMemberIdFacts {
    space_joined_ids: Vec<String>,
    space_invited_ids: Vec<String>,
    child_room_only_ids: Vec<String>,
    child_room_ids: BTreeMap<String, Vec<String>>,
    child_join_input_count: usize,
    child_join_union_count: usize,
    duplicate_child_membership_count: usize,
}

fn classify_space_member_ids<'a, Joined, Invited, ChildRooms, ChildMembers>(
    space_joined_ids: Joined,
    space_invited_ids: Invited,
    child_rooms: ChildRooms,
) -> SpaceMemberIdFacts
where
    Joined: IntoIterator<Item = &'a str>,
    Invited: IntoIterator<Item = &'a str>,
    ChildRooms: IntoIterator<Item = (&'a str, ChildMembers)>,
    ChildMembers: IntoIterator<Item = &'a str>,
{
    let space_joined_ids: BTreeSet<String> = space_joined_ids
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    let space_invited_ids: BTreeSet<String> = space_invited_ids
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    let mut child_room_ids_by_user = BTreeMap::<String, BTreeSet<String>>::new();
    let mut child_membership_count = 0usize;

    for (child_room_id, child_user_ids) in child_rooms {
        for user_id in child_user_ids {
            child_membership_count += 1;
            child_room_ids_by_user
                .entry(user_id.to_owned())
                .or_default()
                .insert(child_room_id.to_owned());
        }
    }

    let child_join_union_count = child_room_ids_by_user.len();
    let duplicate_child_membership_count =
        child_membership_count.saturating_sub(child_join_union_count);
    let child_room_only_ids: Vec<String> = child_room_ids_by_user
        .keys()
        .filter(|user_id| {
            !space_joined_ids.contains(*user_id) && !space_invited_ids.contains(*user_id)
        })
        .cloned()
        .collect();
    let child_room_ids = child_room_ids_by_user
        .into_iter()
        .filter(|(user_id, _)| child_room_only_ids.binary_search(user_id).is_ok())
        .map(|(user_id, room_ids)| (user_id, room_ids.into_iter().collect()))
        .collect();

    SpaceMemberIdFacts {
        space_joined_ids: space_joined_ids.into_iter().collect(),
        space_invited_ids: space_invited_ids.into_iter().collect(),
        child_room_only_ids,
        child_room_ids,
        child_join_input_count: child_membership_count,
        child_join_union_count,
        duplicate_child_membership_count,
    }
}

fn matrix_space_member_entry_from_room_member(
    member: &matrix_sdk::room::RoomMember,
    child_room_ids: Vec<String>,
) -> MatrixSpaceMemberEntry {
    let display_name = member
        .display_name()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned);
    let avatar_url = member.avatar_url().map(ToString::to_string);
    let power_level = matrix_room_member_power_level(member.power_level());

    MatrixSpaceMemberEntry {
        user_id: member.user_id().to_string(),
        display_name,
        avatar_url,
        power_level,
        role: matrix_room_member_role(power_level),
        child_room_ids,
        role_options: Vec::new(),
    }
}

fn merge_local_child_member_profile(
    entries: &mut BTreeMap<String, MatrixSpaceMemberEntry>,
    entry: MatrixSpaceMemberEntry,
) {
    match entries.entry(entry.user_id.clone()) {
        std::collections::btree_map::Entry::Vacant(vacant) => {
            vacant.insert(entry);
        }
        std::collections::btree_map::Entry::Occupied(mut occupied) => {
            let existing = occupied.get_mut();
            if existing.display_name.is_none() {
                existing.display_name = entry.display_name;
            }
            if existing.avatar_url.is_none() {
                existing.avatar_url = entry.avatar_url;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SpaceMemberLookupStatus {
    Observed(usize),
    Failed,
    NotAttempted,
}

impl SpaceMemberLookupStatus {
    fn outcome_token(self) -> &'static str {
        match self {
            Self::Observed(_) => "observed",
            Self::Failed => "lookup_failed",
            Self::NotAttempted => "not_attempted",
        }
    }

    fn availability_token(self) -> &'static str {
        match self {
            Self::Observed(_) => "observed",
            Self::Failed | Self::NotAttempted => "counts_unavailable",
        }
    }

    fn observed_count(self) -> Option<usize> {
        match self {
            Self::Observed(count) => Some(count),
            Self::Failed | Self::NotAttempted => None,
        }
    }
}

pub(super) fn space_members_scope_diagnostic_event(
    space_room_lookup_outcome: &'static str,
    space_joined_lookup: SpaceMemberLookupStatus,
    space_invited_lookup: SpaceMemberLookupStatus,
    child_room_count: Option<usize>,
    complete_child_room_count: Option<usize>,
    incomplete_child_room_count: Option<usize>,
    space_joined_input_count: Option<usize>,
    space_invited_input_count: Option<usize>,
    child_join_input_count: Option<usize>,
    child_join_union_count: Option<usize>,
    duplicate_child_membership_count: Option<usize>,
    child_room_only_count: Option<usize>,
    local_lookup_success_count: usize,
    local_lookup_failure_count: usize,
) -> DiagnosticEvent {
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "sdk.space_members_scope",
        "projection",
    )
    .field(DiagnosticField::token(
        "space_room_lookup_outcome",
        space_room_lookup_outcome,
    ))
    .field(DiagnosticField::token("space_join_filter", "join"))
    .field(DiagnosticField::token("space_invite_filter", "invite"))
    .field(DiagnosticField::token("child_room_join_filter", "join"))
    .field(DiagnosticField::token(
        "space_join_lookup_outcome",
        space_joined_lookup.outcome_token(),
    ))
    .field(DiagnosticField::token(
        "space_invite_lookup_outcome",
        space_invited_lookup.outcome_token(),
    ))
    .field(DiagnosticField::token(
        "space_join_count_availability",
        space_joined_lookup.availability_token(),
    ))
    .field(DiagnosticField::token(
        "space_invite_count_availability",
        space_invited_lookup.availability_token(),
    ))
    .field(DiagnosticField::count(
        "local_member_store_lookup_success_count",
        local_lookup_success_count as u64,
    ))
    .field(DiagnosticField::count(
        "local_member_store_lookup_failure_count",
        local_lookup_failure_count as u64,
    ))
    .field(DiagnosticField::token(
        "child_count_availability",
        if child_room_count.is_some()
            && complete_child_room_count.is_some()
            && incomplete_child_room_count.is_some()
        {
            "observed"
        } else {
            "counts_unavailable"
        },
    ));

    if let Some(count) = space_joined_lookup.observed_count() {
        event = event.field(DiagnosticField::count("space_joined_count", count as u64));
    }
    if let Some(count) = space_invited_lookup.observed_count() {
        event = event.field(DiagnosticField::count("space_invited_count", count as u64));
    }
    if let Some(count) = space_joined_input_count {
        event = event.field(DiagnosticField::count(
            "space_joined_input_count",
            count as u64,
        ));
    }
    if let Some(count) = space_invited_input_count {
        event = event.field(DiagnosticField::count(
            "space_invited_input_count",
            count as u64,
        ));
    }
    if let Some(count) = child_join_input_count {
        event = event.field(DiagnosticField::count(
            "child_join_input_count",
            count as u64,
        ));
    }
    if let Some(count) = child_room_count {
        event = event.field(DiagnosticField::count("child_room_count", count as u64));
    }
    if let Some(count) = complete_child_room_count {
        event = event.field(DiagnosticField::count(
            "complete_child_room_count",
            count as u64,
        ));
    }
    if let Some(count) = incomplete_child_room_count {
        event = event.field(DiagnosticField::count(
            "incomplete_child_room_count",
            count as u64,
        ));
    }
    if let Some(count) = child_join_union_count {
        event = event.field(DiagnosticField::count(
            "child_join_union_count",
            count as u64,
        ));
    }
    if let Some(count) = duplicate_child_membership_count {
        event = event.field(DiagnosticField::count(
            "duplicate_child_membership_count",
            count as u64,
        ));
        event = event.field(DiagnosticField::count("deduplicated_count", count as u64));
    } else {
        event = event.field(DiagnosticField::token(
            "deduplicated_count",
            "counts_unavailable",
        ));
    }
    if let Some(count) = child_room_only_count {
        event = event.field(DiagnosticField::count(
            "child_room_only_count",
            count as u64,
        ));
    }

    if let (Some(joined), Some(invited), Some(child)) = (
        space_joined_input_count,
        space_invited_input_count,
        child_join_input_count,
    ) {
        event = event.field(DiagnosticField::count(
            "input_count",
            (joined + invited + child) as u64,
        ));
    } else {
        event = event.field(DiagnosticField::token("input_count", "counts_unavailable"));
    }
    if let (Some(joined), Some(invited), Some(child)) = (
        space_joined_lookup.observed_count(),
        space_invited_lookup.observed_count(),
        child_room_only_count,
    ) {
        event = event.field(DiagnosticField::count(
            "output_count",
            (joined + invited + child) as u64,
        ));
    } else {
        event = event.field(DiagnosticField::token("output_count", "counts_unavailable"));
    }

    event
        .field(DiagnosticField::token("freshness_status", "not_tracked"))
        .field(DiagnosticField::boolean(
            "network_member_sync_attempted",
            false,
        ))
}

/// Project Space membership from the already available SDK room/store state.
///
/// `JOIN` and `INVITE` are intentionally loaded as separate local-only
/// filters. Child-room members are a deduplicated union of local `JOIN`
/// snapshots, with the Space sets removed afterwards. This function never
/// invokes the SDK's member-syncing `members` method.
pub async fn matrix_space_members_projection(
    session: &MatrixClientSession,
    space_id: &str,
) -> Result<MatrixSpaceMembersProjection, MatrixRoomOperationError> {
    let space_room = match matrix_room(session, space_id) {
        Ok(room) => room,
        Err(error) => {
            record(space_members_scope_diagnostic_event(
                "lookup_failed",
                SpaceMemberLookupStatus::Failed,
                SpaceMemberLookupStatus::NotAttempted,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                0,
                1,
            ));
            return Err(error);
        }
    };

    let mut local_lookup_success_count = 0usize;
    let mut local_lookup_failure_count = 0usize;
    let power_levels = space_room.power_levels_or_default().await;
    let power_levels_revision = space_room
        .get_state_event_static::<RoomPowerLevelsEventContent>()
        .await
        .ok()
        .flatten()
        .and_then(|event| {
            event
                .deserialize()
                .ok()
                .and_then(|state| state.event_id().map(ToString::to_string))
        });
    let can_edit_roles = space_room.state() == matrix_sdk::RoomState::Joined
        && power_levels
            .user_can_send_state(space_room.own_user_id(), StateEventType::RoomPowerLevels);

    let space_joined_members = match space_room
        .members_no_sync(matrix_sdk::RoomMemberships::JOIN)
        .await
    {
        Ok(members) => {
            local_lookup_success_count += 1;
            members
        }
        Err(error) => {
            local_lookup_failure_count += 1;
            record(space_members_scope_diagnostic_event(
                "observed",
                SpaceMemberLookupStatus::Failed,
                SpaceMemberLookupStatus::NotAttempted,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                local_lookup_success_count,
                local_lookup_failure_count,
            ));
            return Err(MatrixRoomOperationError::from_sdk_error(error));
        }
    };
    let space_invited_members = match space_room
        .members_no_sync(matrix_sdk::RoomMemberships::INVITE)
        .await
    {
        Ok(members) => {
            local_lookup_success_count += 1;
            members
        }
        Err(error) => {
            local_lookup_failure_count += 1;
            record(space_members_scope_diagnostic_event(
                "observed",
                SpaceMemberLookupStatus::Observed(space_joined_members.len()),
                SpaceMemberLookupStatus::Failed,
                None,
                None,
                None,
                Some(space_joined_members.len()),
                None,
                None,
                None,
                None,
                None,
                local_lookup_success_count,
                local_lookup_failure_count,
            ));
            return Err(MatrixRoomOperationError::from_sdk_error(error));
        }
    };

    let mut space_joined_by_user = BTreeMap::new();
    for member in &space_joined_members {
        let entry = matrix_space_member_entry_from_room_member(member, Vec::new());
        space_joined_by_user.insert(entry.user_id.clone(), entry);
    }
    let mut space_invited_by_user = BTreeMap::new();
    for member in &space_invited_members {
        let entry = matrix_space_member_entry_from_room_member(member, Vec::new());
        space_invited_by_user.insert(entry.user_id.clone(), entry);
    }

    let child_room_ids = matrix_space_child_room_ids(&space_room).await;
    let mut child_room_memberships = Vec::with_capacity(child_room_ids.len());
    let mut child_profiles = BTreeMap::<String, MatrixSpaceMemberEntry>::new();
    let mut complete_child_room_count = 0usize;
    let mut incomplete_child_room_count = 0usize;

    for child_room_id in &child_room_ids {
        let Ok(parsed_child_room_id) = matrix_sdk::ruma::RoomId::parse(child_room_id) else {
            local_lookup_failure_count += 1;
            incomplete_child_room_count += 1;
            child_room_memberships.push((child_room_id.clone(), Vec::new()));
            continue;
        };
        let Some(child_room) = session.client().get_room(&parsed_child_room_id) else {
            local_lookup_failure_count += 1;
            incomplete_child_room_count += 1;
            child_room_memberships.push((child_room_id.clone(), Vec::new()));
            continue;
        };

        let members_synced = child_room.are_members_synced();
        match child_room
            .members_no_sync(matrix_sdk::RoomMemberships::JOIN)
            .await
        {
            Ok(members) => {
                local_lookup_success_count += 1;
                if members_synced {
                    complete_child_room_count += 1;
                } else {
                    incomplete_child_room_count += 1;
                }
                let mut user_ids = Vec::with_capacity(members.len());
                for member in members {
                    let entry = matrix_space_member_entry_from_room_member(
                        &member,
                        vec![child_room_id.clone()],
                    );
                    user_ids.push(entry.user_id.clone());
                    merge_local_child_member_profile(&mut child_profiles, entry);
                }
                user_ids.sort();
                user_ids.dedup();
                child_room_memberships.push((child_room_id.clone(), user_ids));
            }
            Err(_) => {
                local_lookup_failure_count += 1;
                incomplete_child_room_count += 1;
                child_room_memberships.push((child_room_id.clone(), Vec::new()));
            }
        }
    }

    let child_room_membership_refs: Vec<(&str, Vec<&str>)> = child_room_memberships
        .iter()
        .map(|(room_id, user_ids)| {
            (
                room_id.as_str(),
                user_ids.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    let facts = classify_space_member_ids(
        space_joined_by_user.keys().map(String::as_str),
        space_invited_by_user.keys().map(String::as_str),
        child_room_membership_refs,
    );

    let mut space_joined: Vec<_> = facts
        .space_joined_ids
        .iter()
        .filter_map(|user_id| space_joined_by_user.remove(user_id))
        .collect();
    if can_edit_roles {
        for entry in &mut space_joined {
            entry.role_options =
                direct_space_role_options(&space_room, &power_levels, &entry.user_id);
        }
    }
    let space_invited: Vec<_> = facts
        .space_invited_ids
        .iter()
        .filter_map(|user_id| space_invited_by_user.remove(user_id))
        .collect();
    let child_room_only: Vec<_> = facts
        .child_room_only_ids
        .iter()
        .filter_map(|user_id| {
            let mut entry = child_profiles.remove(user_id)?;
            entry.child_room_ids = facts
                .child_room_ids
                .get(user_id)
                .cloned()
                .unwrap_or_default();
            Some(entry)
        })
        .collect();

    record(space_members_scope_diagnostic_event(
        "observed",
        SpaceMemberLookupStatus::Observed(space_joined.len()),
        SpaceMemberLookupStatus::Observed(space_invited.len()),
        Some(child_room_ids.len()),
        Some(complete_child_room_count),
        Some(incomplete_child_room_count),
        Some(space_joined_members.len()),
        Some(space_invited_members.len()),
        Some(facts.child_join_input_count),
        Some(facts.child_join_union_count),
        Some(facts.duplicate_child_membership_count),
        Some(child_room_only.len()),
        local_lookup_success_count,
        local_lookup_failure_count,
    ));
    let child_room_count = child_room_ids.len();

    Ok(MatrixSpaceMembersProjection {
        space_id: space_id.to_owned(),
        child_room_ids,
        space_joined,
        space_invited,
        child_room_only,
        child_room_profiles: child_profiles.values().cloned().collect(),
        space_joined_input_count: space_joined_members.len(),
        space_invited_input_count: space_invited_members.len(),
        child_join_input_count: facts.child_join_input_count,
        child_join_union_count: facts.child_join_union_count,
        duplicate_child_membership_count: facts.duplicate_child_membership_count,
        child_room_count,
        complete_child_room_count,
        incomplete_child_room_count,
        power_levels_revision,
        can_edit_roles,
    })
}

pub(super) fn matrix_public_room_from_chunk(
    chunk: matrix_sdk::ruma::directory::PublicRoomsChunk,
) -> MatrixPublicRoomDirectoryRoom {
    MatrixPublicRoomDirectoryRoom {
        room_id: chunk.room_id.to_string(),
        canonical_alias: chunk.canonical_alias.map(|alias| alias.to_string()),
        room_type: chunk.room_type.map(|room_type| room_type.to_string()),
        // An unnamed entry stays empty: labelling it here would hardcode
        // English prose and would call an unnamed space a room.
        name: chunk.name.unwrap_or_default(),
        topic: chunk.topic,
        avatar_url: chunk.avatar_url.map(|avatar_url| avatar_url.to_string()),
        joined_members: chunk.num_joined_members.into(),
        world_readable: chunk.world_readable,
        guest_can_join: chunk.guest_can_join,
    }
}

pub(super) async fn matrix_room_settings_snapshot(
    room: &matrix_sdk::Room,
) -> MatrixRoomSettingsSnapshot {
    let power_levels = room.power_levels_or_default().await;
    let own_user_id = room.own_user_id();
    let members = matrix_room_member_summaries(room).await;
    let is_space = room.is_space();
    let child_room_count = if is_space {
        matrix_space_child_room_ids(room).await.len()
    } else {
        0
    };
    record(people_scope_diagnostic_event(
        is_space,
        members.len(),
        child_room_count,
    ));
    let can_edit_settings = power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomName,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomTopic,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomAvatar,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomJoinRules,
    ) && power_levels.user_can_send_state(
        own_user_id,
        matrix_sdk::ruma::events::StateEventType::RoomHistoryVisibility,
    );

    MatrixRoomSettingsSnapshot {
        room_id: room.room_id().to_string(),
        name: room.name(),
        topic: room.topic(),
        avatar_url: room.avatar_url().map(|url| url.to_string()),
        canonical_alias: room.canonical_alias().map(|alias| alias.to_string()),
        alternate_aliases: room
            .alt_aliases()
            .into_iter()
            .map(|alias| alias.to_string())
            .collect(),
        join_rule: room
            .join_rule()
            .as_ref()
            .map(matrix_room_join_rule)
            .unwrap_or(MatrixRoomJoinRule::Invite),
        history_visibility: matrix_room_history_visibility(&room.history_visibility_or_default()),
        permissions: MatrixRoomPermissionFacts {
            can_edit_settings,
            can_edit_roles: power_levels.user_can_send_state(
                own_user_id,
                matrix_sdk::ruma::events::StateEventType::RoomPowerLevels,
            ),
            can_invite: power_levels.user_can_invite(own_user_id),
            can_kick: power_levels.user_can_kick(own_user_id),
            can_ban: power_levels.user_can_ban(own_user_id),
            can_unban: power_levels.user_can_ban(own_user_id),
        },
        members,
    }
}

pub(super) fn people_scope_diagnostic_event(
    is_space: bool,
    direct_member_count: usize,
    child_room_count: usize,
) -> DiagnosticEvent {
    DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "sdk.people_scope",
        "member_snapshot",
    )
    .field(DiagnosticField::token(
        "scope",
        if is_space { "space" } else { "room" },
    ))
    .field(DiagnosticField::token(
        "source",
        if is_space {
            "direct_space_members"
        } else {
            "room_members"
        },
    ))
    .field(DiagnosticField::boolean("aggregated", false))
    .field(DiagnosticField::count(
        "direct_member_count",
        direct_member_count as u64,
    ))
    .field(DiagnosticField::count(
        "child_room_count",
        child_room_count as u64,
    ))
    .field(DiagnosticField::boolean(
        "child_room_members_included",
        false,
    ))
}

async fn matrix_room_member_summaries(room: &matrix_sdk::Room) -> Vec<MatrixRoomMemberSummary> {
    let Ok(members) = room.members(matrix_sdk::RoomMemberships::ACTIVE).await else {
        return Vec::new();
    };
    let encryption = room.client().encryption();
    let mut summaries: Vec<MatrixRoomMemberSummary> = Vec::with_capacity(members.len());
    for member in members {
        let power_level = matrix_room_member_power_level(member.power_level());
        let user_trust = encryption
            .get_user_identity(member.user_id())
            .await
            .ok()
            .flatten()
            .map(matrix_user_trust_state_from_sdk_identity);
        summaries.push(MatrixRoomMemberSummary {
            user_id: member.user_id().to_string(),
            display_name: member.display_name().map(ToOwned::to_owned),
            avatar_url: member.avatar_url().map(ToString::to_string),
            power_level,
            role: matrix_room_member_role(power_level),
            user_trust,
        });
    }
    summaries.sort_by(|left, right| left.user_id.cmp(&right.user_id));
    summaries
}

fn matrix_user_trust_state_from_sdk_identity(
    identity: matrix_sdk::encryption::identities::UserIdentity,
) -> MatrixUserTrustState {
    if identity.has_verification_violation() {
        MatrixUserTrustState::IdentityReset
    } else if identity.is_verified() {
        MatrixUserTrustState::Verified
    } else {
        MatrixUserTrustState::Unverified
    }
}

pub(super) fn room_settings_snapshot_with_member_power_level(
    mut snapshot: MatrixRoomSettingsSnapshot,
    target_user_id: &str,
    power_level: i64,
) -> MatrixRoomSettingsSnapshot {
    if let Some(member) = snapshot
        .members
        .iter_mut()
        .find(|member| member.user_id == target_user_id)
    {
        member.power_level = Some(power_level);
        member.role = matrix_room_member_role(Some(power_level));
    }
    snapshot
}

fn matrix_room_member_power_level(
    power_level: matrix_sdk::ruma::events::room::power_levels::UserPowerLevel,
) -> Option<i64> {
    match power_level {
        matrix_sdk::ruma::events::room::power_levels::UserPowerLevel::Infinite => None,
        matrix_sdk::ruma::events::room::power_levels::UserPowerLevel::Int(value) => {
            Some(value.into())
        }
        _ => None,
    }
}

pub(super) fn matrix_room_member_role(power_level: Option<i64>) -> MatrixRoomMemberRole {
    match power_level {
        None => MatrixRoomMemberRole::Creator,
        Some(level) if level >= 100 => MatrixRoomMemberRole::Administrator,
        Some(level) if level >= 50 => MatrixRoomMemberRole::Moderator,
        Some(_) => MatrixRoomMemberRole::User,
    }
}

fn finite_power_level(power_level: UserPowerLevel) -> Option<i64> {
    match power_level {
        UserPowerLevel::Infinite => None,
        UserPowerLevel::Int(value) => Some(value.into()),
        _ => None,
    }
}

fn direct_space_role_options(
    room: &matrix_sdk::Room,
    power_levels: &matrix_sdk::ruma::events::room::power_levels::RoomPowerLevels,
    target_user_id: &str,
) -> Vec<MatrixSpaceMemberRoleOption> {
    let Ok(target_user_id) = matrix_sdk::ruma::UserId::parse(target_user_id) else {
        return Vec::new();
    };
    let own_power = power_levels.for_user(room.own_user_id());
    let target_power = power_levels.for_user(&target_user_id);
    let can_edit_roles =
        power_levels.user_can_send_state(room.own_user_id(), StateEventType::RoomPowerLevels);
    role_options_for_powers(
        own_power,
        target_power,
        &target_user_id == room.own_user_id(),
        can_edit_roles,
    )
}

fn role_options_for_powers(
    own_power: UserPowerLevel,
    target_power: UserPowerLevel,
    target_is_self: bool,
    can_edit_roles: bool,
) -> Vec<MatrixSpaceMemberRoleOption> {
    if target_is_self || !can_edit_roles {
        return Vec::new();
    }
    // No one may edit a creator/infinite target, and a finite caller must be
    // strictly above both the existing target and every proposed level.
    if target_power == UserPowerLevel::Infinite || own_power <= target_power {
        return Vec::new();
    }
    let Some(current) = finite_power_level(target_power) else {
        return Vec::new();
    };
    [0_i64, 50, 100]
        .into_iter()
        .filter(|candidate| {
            *candidate != current
                && match own_power {
                    UserPowerLevel::Infinite => true,
                    UserPowerLevel::Int(value) => *candidate < i64::from(value),
                    _ => false,
                }
        })
        .map(|power_level| MatrixSpaceMemberRoleOption {
            power_level,
            role: matrix_room_member_role(Some(power_level)),
            requires_confirmation: current >= 100 || power_level >= 100,
        })
        .collect()
}

pub(super) fn room_settings_snapshot_with_change(
    mut snapshot: MatrixRoomSettingsSnapshot,
    change: &MatrixRoomSettingChange,
) -> MatrixRoomSettingsSnapshot {
    match change {
        MatrixRoomSettingChange::Name(name) => {
            snapshot.name = name.clone();
        }
        MatrixRoomSettingChange::Topic(topic) => {
            snapshot.topic = topic.clone();
        }
        MatrixRoomSettingChange::AvatarUrl(avatar_url) => {
            snapshot.avatar_url = avatar_url.clone();
        }
        MatrixRoomSettingChange::JoinRule(join_rule) => {
            snapshot.join_rule = *join_rule;
        }
        MatrixRoomSettingChange::HistoryVisibility(history_visibility) => {
            snapshot.history_visibility = *history_visibility;
        }
    }
    snapshot
}

fn matrix_room_join_rule(
    join_rule: &matrix_sdk::ruma::events::room::join_rules::JoinRule,
) -> MatrixRoomJoinRule {
    use matrix_sdk::ruma::events::room::join_rules::JoinRule;
    match join_rule {
        JoinRule::Public => MatrixRoomJoinRule::Public,
        JoinRule::Invite => MatrixRoomJoinRule::Invite,
        JoinRule::Knock => MatrixRoomJoinRule::Knock,
        JoinRule::Restricted(_) | JoinRule::KnockRestricted(_) => MatrixRoomJoinRule::Restricted,
        JoinRule::Private => MatrixRoomJoinRule::Private,
        _ => MatrixRoomJoinRule::Invite,
    }
}

pub(super) fn sdk_join_rule_for_update(
    join_rule: MatrixRoomJoinRule,
) -> Result<matrix_sdk::ruma::events::room::join_rules::JoinRule, MatrixRoomOperationError> {
    use matrix_sdk::ruma::events::room::join_rules::JoinRule;
    match join_rule {
        MatrixRoomJoinRule::Public => Ok(JoinRule::Public),
        MatrixRoomJoinRule::Invite => Ok(JoinRule::Invite),
        MatrixRoomJoinRule::Knock => Ok(JoinRule::Knock),
        MatrixRoomJoinRule::Private => Ok(JoinRule::Private),
        MatrixRoomJoinRule::Restricted => Err(MatrixRoomOperationError::InvalidRoomSetting),
    }
}

fn matrix_room_history_visibility(
    history_visibility: &matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility,
) -> MatrixRoomHistoryVisibility {
    use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
    match history_visibility {
        HistoryVisibility::WorldReadable => MatrixRoomHistoryVisibility::WorldReadable,
        HistoryVisibility::Shared => MatrixRoomHistoryVisibility::Shared,
        HistoryVisibility::Invited => MatrixRoomHistoryVisibility::Invited,
        HistoryVisibility::Joined => MatrixRoomHistoryVisibility::Joined,
        _ => MatrixRoomHistoryVisibility::Shared,
    }
}

pub(super) fn sdk_history_visibility(
    history_visibility: MatrixRoomHistoryVisibility,
) -> matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility {
    use matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility;
    match history_visibility {
        MatrixRoomHistoryVisibility::WorldReadable => HistoryVisibility::WorldReadable,
        MatrixRoomHistoryVisibility::Shared => HistoryVisibility::Shared,
        MatrixRoomHistoryVisibility::Invited => HistoryVisibility::Invited,
        MatrixRoomHistoryVisibility::Joined => HistoryVisibility::Joined,
    }
}

pub(super) fn non_empty_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

pub(super) fn matrix_room_operation_failure_kind(
    error: &matrix_sdk::Error,
) -> MatrixRoomOperationFailureKind {
    match error {
        matrix_sdk::Error::AuthenticationRequired => {
            MatrixRoomOperationFailureKind::AuthenticationRequired
        }
        matrix_sdk::Error::WrongRoomState(_) => MatrixRoomOperationFailureKind::WrongRoomState,
        matrix_sdk::Error::Http(error) => {
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                )
            {
                MatrixRoomOperationFailureKind::Forbidden
            } else {
                MatrixRoomOperationFailureKind::Http
            }
        }
        matrix_sdk::Error::BadCryptoStoreState
        | matrix_sdk::Error::NoOlmMachine
        | matrix_sdk::Error::CryptoStoreError(_)
        | matrix_sdk::Error::OlmError(_)
        | matrix_sdk::Error::MegolmError(_)
        | matrix_sdk::Error::DecryptorError(_) => MatrixRoomOperationFailureKind::Encryption,
        matrix_sdk::Error::StateStore(_)
        | matrix_sdk::Error::EventCacheStore(_)
        | matrix_sdk::Error::MediaStore(_) => MatrixRoomOperationFailureKind::Store,
        matrix_sdk::Error::SecureBackupRequired => {
            MatrixRoomOperationFailureKind::SecureBackupRequired
        }
        matrix_sdk::Error::EncryptionReadiness(_) => {
            MatrixRoomOperationFailureKind::EncryptionReadiness
        }
        matrix_sdk::Error::SerdeJson(_)
        | matrix_sdk::Error::Io(_)
        | matrix_sdk::Error::CrossProcessLockError(_)
        | matrix_sdk::Error::Identifier(_)
        | matrix_sdk::Error::Url(_)
        | matrix_sdk::Error::SlidingSync(_)
        | matrix_sdk::Error::MultipleSessionCallbacks
        | matrix_sdk::Error::OAuth(_)
        | matrix_sdk::Error::ConcurrentRequestFailed
        | matrix_sdk::Error::UnknownError(_)
        | matrix_sdk::Error::EventCache(_)
        | matrix_sdk::Error::SendQueueWedgeError(_)
        | matrix_sdk::Error::BackupNotEnabled
        | matrix_sdk::Error::CantIgnoreLoggedInUser
        | matrix_sdk::Error::Media(_)
        | matrix_sdk::Error::ReplyError(_)
        | matrix_sdk::Error::PowerLevels(_)
        | matrix_sdk::Error::Timeout
        | matrix_sdk::Error::InsufficientData => MatrixRoomOperationFailureKind::Sdk,
        _ => MatrixRoomOperationFailureKind::Sdk,
    }
}

pub(super) fn timeline_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<matrix_sdk::Room, MatrixTimelineError> {
    let room_id =
        matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixTimelineError::InvalidRoomId)?;
    session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixTimelineError::RoomUnavailable)
}

pub(super) fn matrix_timeline_updates_from_diffs(
    room_id: &str,
    diffs: Vec<eyeball_im::VectorDiff<Arc<matrix_sdk_ui::timeline::TimelineItem>>>,
) -> Vec<MatrixTimelineUpdate> {
    let mut updates = Vec::new();
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::Append { values }
            | eyeball_im::VectorDiff::Reset { values } => {
                updates.extend(
                    values
                        .iter()
                        .filter_map(|item| matrix_timeline_update_from_ui(room_id, item)),
                );
            }
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => {
                if let Some(update) = matrix_timeline_update_from_ui(room_id, &value) {
                    updates.push(update);
                }
            }
            eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack
            | eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. } => {}
        }
    }
    updates
}

pub(super) fn matrix_timeline_update_from_ui(
    room_id: &str,
    item: &matrix_sdk_ui::timeline::TimelineItem,
) -> Option<MatrixTimelineUpdate> {
    let event = item.as_event()?;
    let event_id = event.event_id()?.to_string();
    match event.content().as_message() {
        Some(content) => Some(MatrixTimelineUpdate::Upsert(MatrixTimelineItem {
            room_id: room_id.to_owned(),
            event_id,
            sender: event.sender().to_string(),
            timestamp_ms: event.timestamp().0.into(),
            body: content.body().to_owned(),
        })),
        None => Some(MatrixTimelineUpdate::Remove {
            room_id: room_id.to_owned(),
            event_id,
        }),
    }
}

async fn matrix_room_list_snapshot_from_diffs(
    direct_targets_by_room: Option<&MatrixDirectTargetsByRoom>,
    diffs: Vec<eyeball_im::VectorDiff<matrix_sdk_ui::room_list_service::RoomListItem>>,
) -> MatrixRoomListSnapshot {
    let mut items = Vec::new();
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::Append { values }
            | eyeball_im::VectorDiff::Reset { values } => {
                items.extend(values.into_iter());
            }
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => {
                items.push(value);
            }
            eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack
            | eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. } => {}
        }
    }

    matrix_room_list_snapshot_from_items(direct_targets_by_room, items).await
}

async fn matrix_room_list_snapshot_from_items(
    direct_targets_by_room: Option<&MatrixDirectTargetsByRoom>,
    items: Vec<matrix_sdk_ui::room_list_service::RoomListItem>,
) -> MatrixRoomListSnapshot {
    matrix_room_list_snapshot_from_rooms(
        direct_targets_by_room,
        items.into_iter().map(|item| item.into_inner()),
    )
    .await
}

async fn matrix_room_list_snapshot_from_rooms(
    direct_targets_by_room: Option<&MatrixDirectTargetsByRoom>,
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> MatrixRoomListSnapshot {
    let mut snapshot = MatrixRoomListSnapshot::default();
    let mut user_profiles = BTreeMap::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Joined {
            continue;
        }

        let room_id = room.room_id().to_string();
        let display_name = room
            .cached_display_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| room_id.clone());

        if room.is_space() {
            let child_room_ids = matrix_space_child_room_ids(&room).await;
            let member_user_ids = matrix_space_member_user_ids_no_sync(&room).await;
            snapshot.spaces.push(MatrixRoomListSpace {
                space_id: room_id,
                display_name,
                avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
                child_room_ids,
                member_user_ids,
            });
            continue;
        }

        let unread_notifications = room.unread_notification_counts();
        let notification_count = unread_notifications.notification_count.into();
        let highlight_count = unread_notifications.highlight_count.into();
        let is_marked_unread = room.is_marked_unread();
        let unread_messages = room.num_unread_messages();
        // Keep raw unread messages separate from notification and manual-unread
        // projections. The state layer derives activity from all four fields;
        // a manual mark must not fabricate a Dock badge count.
        let unread_count = unread_messages;

        let parent_space_ids = matrix_parent_space_ids(&room).await;
        let tags = matrix_room_tags(&room).await;

        let is_dm = match direct_targets_by_room {
            Some(direct_targets_by_room) => direct_targets_by_room.contains_key(&room_id),
            None => {
                if !room.direct_targets().is_empty() {
                    true
                } else {
                    room.is_direct().await.unwrap_or_else(|_| room.is_dm())
                }
            }
        };
        let empty_direct_targets_by_room = BTreeMap::new();
        let dm_targets_by_room = if is_dm {
            direct_targets_by_room.unwrap_or(&empty_direct_targets_by_room)
        } else {
            &empty_direct_targets_by_room
        };
        let dm_user_ids =
            matrix_room_list_dm_user_ids(&room, dm_targets_by_room, is_dm, &mut user_profiles)
                .await;
        let joined_members = room.joined_members_count();

        let is_encrypted = room
            .latest_encryption_state()
            .await
            .map(|state| state.is_encrypted())
            .unwrap_or(false);
        let (latest_event, conversation_activity) =
            matrix_room_latest_event_projection(&room).await;
        let fully_read_event_id = matrix_room_fully_read_event_id(&room).await;
        let private_read_receipt_event_id = matrix_room_private_read_receipt_event_id(&room).await;
        let recency_stamp = room.recency_stamp().map(Into::into);
        if unread_count > 0 || notification_count > 0 || highlight_count > 0 || is_marked_unread {
            trace_sdk_unread_snapshot(SdkUnreadTrace {
                unread_messages,
                unread_count,
                notification_count,
                highlight_count,
                marked_unread: is_marked_unread,
                latest_event: &latest_event,
                fully_read_event_id: fully_read_event_id.as_deref(),
                private_read_receipt_event_id: private_read_receipt_event_id.as_deref(),
                recency_stamp_present: recency_stamp.is_some(),
                conversation_activity,
            });
        }

        snapshot.rooms.push(matrix_room_list_room_from_counts(
            room_id,
            display_name,
            room.avatar_url().map(|uri| uri.to_string()),
            is_dm,
            dm_user_ids,
            tags,
            notification_count,
            highlight_count,
            unread_count,
            is_marked_unread,
            recency_stamp,
            conversation_activity,
            latest_event,
            fully_read_event_id,
            private_read_receipt_event_id,
            parent_space_ids,
            is_encrypted,
            joined_members,
        ));
    }
    snapshot.user_profiles = user_profiles.into_values().collect();
    snapshot
}

async fn matrix_direct_account_data_targets_by_room(
    client: &matrix_sdk::Client,
) -> MatrixDirectTargetsByRoom {
    let Some(targets_by_room) = client
        .account()
        .account_data::<DirectEventContent>()
        .await
        .ok()
        .flatten()
        .and_then(|raw_content| raw_content.deserialize().ok())
        .map(|content| direct_account_data_targets_by_room(&content))
    else {
        return client
            .account()
            .fetch_account_data_static::<DirectEventContent>()
            .await
            .ok()
            .flatten()
            .and_then(|raw_content| raw_content.deserialize().ok())
            .map(|content| direct_account_data_targets_by_room(&content))
            .unwrap_or_default();
    };
    targets_by_room
}

async fn matrix_room_list_dm_user_ids(
    room: &matrix_sdk::Room,
    direct_targets_by_room: &MatrixDirectTargetsByRoom,
    is_dm: bool,
    user_profiles: &mut BTreeMap<String, MatrixUserProfile>,
) -> Vec<String> {
    let room_id = room.room_id().to_string();
    let own_user_id = room.own_user_id().to_string();
    let mut candidate_user_ids = if let Some(targets) = direct_targets_by_room.get(&room_id) {
        targets.clone()
    } else {
        let cached_direct_targets: Vec<String> = room
            .direct_targets()
            .into_iter()
            .map(|user_id| user_id.to_string())
            .collect();
        if is_dm && !cached_direct_targets.is_empty() {
            cached_direct_targets
        } else if is_dm {
            room.heroes()
                .into_iter()
                .map(|hero| hero.user_id.to_string())
                .filter(|user_id| user_id != &own_user_id)
                .collect()
        } else {
            Vec::new()
        }
    };

    candidate_user_ids.sort();
    candidate_user_ids.dedup();

    let mut dm_user_ids = Vec::new();
    for candidate_user_id in candidate_user_ids {
        if candidate_user_id == own_user_id {
            continue;
        }

        let Ok(candidate_user_id) =
            matrix_sdk::ruma::OwnedUserId::try_from(candidate_user_id.as_str())
        else {
            continue;
        };

        let candidate_user_id_string = candidate_user_id.to_string();
        dm_user_ids.push(candidate_user_id_string.clone());

        if let Some(member) = room
            .get_member_no_sync(&candidate_user_id)
            .await
            .ok()
            .flatten()
        {
            user_profiles
                .entry(candidate_user_id_string.clone())
                .or_insert_with(|| MatrixUserProfile {
                    user_id: candidate_user_id_string,
                    display_name: member.display_name().map(ToOwned::to_owned),
                    avatar_mxc_uri: member.avatar_url().map(ToString::to_string),
                });
        }
    }

    dm_user_ids.sort();
    dm_user_ids.dedup();
    dm_user_ids
}

async fn matrix_space_member_user_ids_no_sync(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(members) = room
        .members_no_sync(matrix_sdk::RoomMemberships::JOIN)
        .await
    else {
        return Vec::new();
    };
    let mut user_ids: Vec<String> = members
        .into_iter()
        .map(|member| member.user_id().to_string())
        .collect();
    user_ids.sort();
    user_ids.dedup();
    user_ids
}

async fn matrix_invite_previews_from_rooms(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> Vec<MatrixInvitePreview> {
    let mut invites = Vec::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Invited {
            continue;
        }

        let display_name = room
            .display_name()
            .await
            .ok()
            .map(|name| name.to_string())
            .or_else(|| room.name())
            .unwrap_or_else(|| "Invite".to_owned());
        let inviter = room
            .invite_details()
            .await
            .ok()
            .and_then(|details| details.inviter);
        let inviter_display_name = inviter
            .as_ref()
            .and_then(|inviter| inviter.display_name().map(ToOwned::to_owned));
        let inviter_user_id = inviter.map(|inviter| inviter.user_id().to_string());
        let is_dm = room.is_direct().await.unwrap_or(false);

        invites.push(MatrixInvitePreview {
            room_id: room.room_id().to_string(),
            display_name,
            avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
            topic: room.topic(),
            inviter_display_name,
            inviter_user_id,
            is_dm,
        });
    }
    invites
}

fn room_attention_unread_count(
    notification_count: u64,
    unread_messages: u64,
    is_marked_unread: bool,
) -> u64 {
    let unread_count = notification_count.max(unread_messages);
    if unread_count == 0 && is_marked_unread {
        1
    } else {
        unread_count
    }
}

pub(super) fn matrix_room_list_room_from_counts(
    room_id: String,
    display_name: String,
    avatar_mxc_uri: Option<String>,
    is_dm: bool,
    dm_user_ids: Vec<String>,
    tags: MatrixRoomTags,
    notification_count: u64,
    highlight_count: u64,
    unread_messages: u64,
    marked_unread: bool,
    recency_stamp: Option<u64>,
    conversation_activity: Option<MatrixConversationActivity>,
    latest_event: Option<MatrixRoomLatestEventSummary>,
    fully_read_event_id: Option<String>,
    private_read_receipt_event_id: Option<String>,
    parent_space_ids: Vec<String>,
    is_encrypted: bool,
    joined_members: u64,
) -> MatrixRoomListRoom {
    let marker_covers_latest =
        !marked_unread && read_marker_matches_latest(&latest_event, &fully_read_event_id);
    let marker_covers_latest = marker_covers_latest
        || (!marked_unread
            && read_marker_matches_latest(&latest_event, &private_read_receipt_event_id));
    let (unread_count, notification_count, highlight_count) = if marker_covers_latest {
        (0, 0, 0)
    } else {
        (unread_messages, notification_count, highlight_count)
    };
    MatrixRoomListRoom {
        room_id,
        display_name,
        avatar_mxc_uri,
        is_dm,
        dm_user_ids,
        tags,
        unread_count,
        notification_count,
        highlight_count,
        marked_unread,
        recency_stamp,
        conversation_activity,
        latest_event,
        parent_space_ids,
        is_encrypted,
        joined_members,
    }
}

fn read_marker_matches_latest(
    latest_event: &Option<MatrixRoomLatestEventSummary>,
    read_marker_event_id: &Option<String>,
) -> bool {
    latest_event.as_ref().is_some_and(|event| {
        !event.is_redacted
            && !event.event_id.trim().is_empty()
            && matches!(
                event.event_type.as_deref(),
                Some("m.room.message" | "m.room.encrypted")
            )
            && !matches!(
                event.relation_type.as_deref(),
                Some("m.replace" | "m.annotation")
            )
            && read_marker_event_id.as_deref() == Some(event.event_id.as_str())
    })
}

async fn matrix_room_fully_read_event_id(room: &matrix_sdk::Room) -> Option<String> {
    room.account_data_static::<FullyReadEventContent>()
        .await
        .ok()
        .flatten()?
        .deserialize()
        .ok()
        .map(|event| event.content.event_id.to_string())
}

async fn matrix_room_private_read_receipt_event_id(room: &matrix_sdk::Room) -> Option<String> {
    let user_id = room.client().user_id()?.to_owned();
    room.load_user_receipt(
        ReceiptType::ReadPrivate,
        ReceiptThread::Unthreaded,
        &user_id,
    )
    .await
    .ok()
    .flatten()
    .map(|(event_id, _)| event_id.to_string())
}

fn matrix_timeline_event_type(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<String> {
    timeline_event
        .raw()
        .get_field::<String>("type")
        .ok()
        .flatten()
}

fn matrix_timeline_event_relation(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> (Option<String>, Option<String>) {
    let Ok(Some(content)) = timeline_event
        .raw()
        .get_field::<serde_json::Value>("content")
    else {
        return (None, None);
    };
    let Some(relates_to) = content.get("m.relates_to") else {
        return (None, None);
    };
    let relation_event_id = relates_to
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            relates_to
                .get("m.in_reply_to")
                .and_then(|reply| reply.get("event_id"))
                .and_then(serde_json::Value::as_str)
        })
        .map(ToOwned::to_owned);
    let relation_type = relates_to
        .get("rel_type")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            relation_event_id
                .as_ref()
                .map(|_| "m.in_reply_to".to_owned())
        });
    (relation_type, relation_event_id)
}

async fn matrix_room_latest_remote_event_projection(
    room: &matrix_sdk::Room,
    timeline_event: matrix_sdk::deserialized_responses::TimelineEvent,
    cached_conversation_activity: Option<MatrixConversationActivity>,
) -> (
    Option<MatrixRoomLatestEventSummary>,
    Option<MatrixConversationActivity>,
) {
    let is_redacted = matrix_timeline_event_is_redacted(&timeline_event);
    let replacement_target = {
        let (relation_type, relation_event_id) = matrix_timeline_event_relation(&timeline_event);
        (relation_type.as_deref() == Some("m.replace")).then_some(relation_event_id)
    };

    let (identity_event, preview_content, facts_content) = if is_redacted {
        let content =
            matrix_sdk_ui::timeline::TimelineItemContent::from_event(room, timeline_event.clone())
                .await;
        (timeline_event, content.clone(), content)
    } else if let Some(relation_event_id) = replacement_target.flatten() {
        let Some(original_event) =
            matrix_room_event_in_memory_by_id(room, &relation_event_id).await
        else {
            return (None, cached_conversation_activity);
        };
        if matrix_timeline_event_is_redacted(&original_event) {
            return (None, cached_conversation_activity);
        }
        let original_event_type = matrix_timeline_event_type(&original_event);
        let (original_relation_type, _) = matrix_timeline_event_relation(&original_event);
        if matrix_conversation_activity_source(
            original_event_type.as_deref().unwrap_or_default(),
            original_relation_type.as_deref(),
        )
        .is_none()
        {
            return (None, cached_conversation_activity);
        }

        let facts_content =
            matrix_sdk_ui::timeline::TimelineItemContent::from_event(room, original_event.clone())
                .await;
        let preview_content =
            matrix_sdk_ui::timeline::TimelineItemContent::from_event(room, timeline_event.clone())
                .await;
        (original_event, preview_content, facts_content)
    } else {
        let content =
            matrix_sdk_ui::timeline::TimelineItemContent::from_event(room, timeline_event.clone())
                .await;
        (timeline_event, content.clone(), content)
    };

    let Some(event_id) = identity_event
        .event_id()
        .map(|event_id| event_id.to_string())
    else {
        return (None, cached_conversation_activity);
    };
    let sender = identity_event.sender();
    let timestamp_ms = identity_event
        .timestamp()
        .map(|timestamp| u64::from(timestamp.get()))
        .unwrap_or(0);
    let event_type = matrix_timeline_event_type(&identity_event);
    let (relation_type, relation_event_id) = matrix_timeline_event_relation(&identity_event);
    let content_converted = facts_content.is_some();
    let is_threaded = facts_content
        .as_ref()
        .is_some_and(|content| content.thread_root().is_some());
    let is_reply = facts_content
        .as_ref()
        .is_some_and(|content| content.in_reply_to().is_some());
    let has_thread_summary = facts_content
        .as_ref()
        .is_some_and(|content| content.thread_summary().is_some());
    let has_reactions = facts_content
        .as_ref()
        .and_then(|content| content.reactions())
        .is_some_and(|reactions| !reactions.is_empty());
    let (sender_label, sender_avatar_mxc_uri) = match sender.as_ref() {
        Some(sender) => matrix_room_member_display(room, sender).await,
        None => (None, None),
    };
    let conversation_activity = if is_redacted {
        None
    } else {
        matrix_conversation_activity_source(
            event_type.as_deref().unwrap_or_default(),
            relation_type.as_deref(),
        )
        .map(|source| MatrixConversationActivity {
            timestamp_ms,
            source,
        })
    };
    let latest_event = MatrixRoomLatestEventSummary {
        event_id,
        sender_id: sender.map(|sender| sender.to_string()),
        sender_label,
        sender_avatar_mxc_uri,
        preview: (!is_redacted)
            .then(|| {
                preview_content
                    .as_ref()
                    .and_then(matrix_latest_event_preview)
            })
            .flatten(),
        timestamp_ms,
        event_type,
        relation_type,
        relation_event_id,
        content_converted,
        is_threaded,
        is_reply,
        has_thread_summary,
        has_reactions,
        is_redacted,
    };
    (
        Some(latest_event),
        newest_conversation_activity(cached_conversation_activity, conversation_activity),
    )
}

async fn matrix_room_event_in_memory_by_id(
    room: &matrix_sdk::Room,
    event_id: &str,
) -> Option<matrix_sdk::deserialized_responses::TimelineEvent> {
    let (event_cache, _drop_handles) = room.event_cache().await.ok()?;
    event_cache
        .rfind_map_event_in_memory_by(|event| {
            event
                .event_id()
                .as_deref()
                .is_some_and(|candidate| candidate.as_str() == event_id)
                .then(|| event.clone())
        })
        .await
        .ok()
        .flatten()
}

fn matrix_local_latest_event_relation_type(
    content: &matrix_sdk::store::SerializableEventContent,
) -> Option<String> {
    let (raw_content, _) = content.raw();
    raw_content
        .get_field::<serde_json::Value>("m.relates_to")
        .ok()
        .flatten()
        .and_then(|relates_to| {
            relates_to
                .get("rel_type")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    relates_to
                        .get("m.in_reply_to")
                        .and_then(|reply| reply.get("event_id"))
                        .is_some()
                        .then(|| "m.in_reply_to".to_owned())
                })
        })
}

fn matrix_local_latest_event_projection(
    event_id: Option<&str>,
    value: &matrix_sdk::latest_events::LocalLatestEventValue,
    sender_id: Option<String>,
    cached_conversation_activity: Option<MatrixConversationActivity>,
) -> (
    Option<MatrixRoomLatestEventSummary>,
    Option<MatrixConversationActivity>,
) {
    let (raw_content, event_type) = value.content.raw();
    let relation_type = matrix_local_latest_event_relation_type(&value.content);
    if relation_type.as_deref() == Some("m.replace") {
        return (None, cached_conversation_activity);
    }
    let timestamp_ms = u64::from(value.timestamp.get());
    let conversation_activity =
        matrix_conversation_activity_source(event_type, relation_type.as_deref()).map(|source| {
            MatrixConversationActivity {
                timestamp_ms,
                source,
            }
        });
    let Some(event_id) = event_id else {
        return (
            None,
            newest_conversation_activity(cached_conversation_activity, conversation_activity),
        );
    };
    let latest_event = MatrixRoomLatestEventSummary {
        event_id: event_id.to_owned(),
        sender_id: sender_id.clone(),
        sender_label: sender_id,
        sender_avatar_mxc_uri: None,
        preview: matrix_local_latest_event_preview(&value.content),
        timestamp_ms,
        event_type: Some(event_type.to_owned()),
        relation_type,
        relation_event_id: None,
        content_converted: false,
        is_threaded: false,
        is_reply: false,
        has_thread_summary: false,
        has_reactions: false,
        is_redacted: false,
    };
    (
        Some(latest_event),
        newest_conversation_activity(cached_conversation_activity, conversation_activity),
    )
}

async fn matrix_room_latest_event_projection(
    room: &matrix_sdk::Room,
) -> (
    Option<MatrixRoomLatestEventSummary>,
    Option<MatrixConversationActivity>,
) {
    let client = room.client();
    if client.event_cache().has_subscribed() {
        let latest_events = client.latest_events().await;
        let _ = latest_events.listen_to_room(room.room_id()).await;
    }
    let cached_conversation_activity = matrix_room_cached_conversation_activity(room).await;
    let latest_event = room.latest_event();
    match latest_event {
        matrix_sdk::latest_events::LatestEventValue::Remote(timeline_event) => {
            matrix_room_latest_remote_event_projection(
                room,
                timeline_event,
                cached_conversation_activity,
            )
            .await
        }
        matrix_sdk::latest_events::LatestEventValue::LocalHasBeenSent { event_id, value } => {
            matrix_local_latest_event_projection(
                Some(event_id.as_str()),
                &value,
                room.client().user_id().map(|user_id| user_id.to_string()),
                cached_conversation_activity,
            )
        }
        matrix_sdk::latest_events::LatestEventValue::LocalIsSending(value) => {
            matrix_local_latest_event_projection(
                None,
                &value,
                room.client().user_id().map(|user_id| user_id.to_string()),
                cached_conversation_activity,
            )
        }
        matrix_sdk::latest_events::LatestEventValue::None
        | matrix_sdk::latest_events::LatestEventValue::RemoteInvite { .. }
        | matrix_sdk::latest_events::LatestEventValue::LocalCannotBeSent(_) => {
            (None, cached_conversation_activity)
        }
    }
}

fn matrix_timeline_event_is_redacted(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> bool {
    #[derive(serde::Deserialize)]
    struct Unsigned {
        redacted_because: Option<serde_json::Value>,
    }

    match timeline_event.raw().get_field::<Unsigned>("unsigned") {
        Ok(Some(unsigned)) => unsigned.redacted_because.is_some(),
        Ok(None) => false,
        // A malformed unsigned block cannot be trusted as a display/read
        // anchor; fail closed without logging the private event.
        Err(_) => true,
    }
}

async fn matrix_room_cached_conversation_activity(
    room: &matrix_sdk::Room,
) -> Option<MatrixConversationActivity> {
    let (event_cache, _drop_handles) = room.event_cache().await.ok()?;
    event_cache
        .rfind_map_event_in_memory_by(matrix_conversation_activity_from_timeline_event)
        .await
        .ok()
        .flatten()
}

fn matrix_conversation_activity_from_timeline_event(
    timeline_event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<MatrixConversationActivity> {
    if matrix_timeline_event_is_redacted(timeline_event) {
        return None;
    }
    let event_type = matrix_timeline_event_type(timeline_event)?;
    let (relation_type, _) = matrix_timeline_event_relation(timeline_event);
    let source = matrix_conversation_activity_source(&event_type, relation_type.as_deref())?;
    let timestamp_ms = timeline_event
        .timestamp()
        .map(|timestamp| u64::from(timestamp.get()))?;
    Some(MatrixConversationActivity {
        timestamp_ms,
        source,
    })
}

pub(super) fn newest_conversation_activity(
    left: Option<MatrixConversationActivity>,
    right: Option<MatrixConversationActivity>,
) -> Option<MatrixConversationActivity> {
    match (left, right) {
        (Some(left), Some(right)) if right.timestamp_ms > left.timestamp_ms => Some(right),
        (Some(left), _) => Some(left),
        (None, right) => right,
    }
}

pub(super) fn matrix_conversation_activity_source(
    event_type: &str,
    relation_type: Option<&str>,
) -> Option<MatrixConversationActivitySource> {
    if matches!(relation_type, Some("m.replace" | "m.annotation")) {
        return None;
    }
    if relation_type == Some("m.thread") {
        return matches!(event_type, "m.room.message" | "m.room.encrypted")
            .then_some(MatrixConversationActivitySource::ThreadReply);
    }
    match event_type {
        "m.room.message" => Some(MatrixConversationActivitySource::Message),
        "m.room.encrypted" => Some(MatrixConversationActivitySource::EncryptedMessage),
        _ => None,
    }
}

async fn matrix_room_member_display(
    room: &matrix_sdk::Room,
    user_id: &matrix_sdk::ruma::UserId,
) -> (Option<String>, Option<String>) {
    match room.get_member_no_sync(user_id).await {
        Ok(Some(member)) => (
            member.display_name().map(ToOwned::to_owned),
            member.avatar_url().map(ToString::to_string),
        ),
        Ok(None) | Err(_) => (None, None),
    }
}

fn matrix_latest_event_preview(
    content: &matrix_sdk_ui::timeline::TimelineItemContent,
) -> Option<String> {
    match content {
        matrix_sdk_ui::timeline::TimelineItemContent::MsgLike(msglike) => match &msglike.kind {
            matrix_sdk_ui::timeline::MsgLikeKind::Message(message) => {
                Some(message.body().to_owned())
            }
            matrix_sdk_ui::timeline::MsgLikeKind::UnableToDecrypt(_) => {
                Some("Unable to decrypt message".to_owned())
            }
            matrix_sdk_ui::timeline::MsgLikeKind::Redacted => Some("Message deleted".to_owned()),
            matrix_sdk_ui::timeline::MsgLikeKind::Sticker(_)
            | matrix_sdk_ui::timeline::MsgLikeKind::Poll(_)
            | matrix_sdk_ui::timeline::MsgLikeKind::Other(_)
            | matrix_sdk_ui::timeline::MsgLikeKind::LiveLocation(_) => content.event_type_str(),
        },
        matrix_sdk_ui::timeline::TimelineItemContent::MembershipChange(_)
        | matrix_sdk_ui::timeline::TimelineItemContent::ProfileChange(_)
        | matrix_sdk_ui::timeline::TimelineItemContent::OtherState(_)
        | matrix_sdk_ui::timeline::TimelineItemContent::FailedToParseMessageLike { .. }
        | matrix_sdk_ui::timeline::TimelineItemContent::FailedToParseState { .. }
        | matrix_sdk_ui::timeline::TimelineItemContent::CallInvite
        | matrix_sdk_ui::timeline::TimelineItemContent::RtcNotification { .. } => {
            content.event_type_str()
        }
    }
}

fn matrix_local_latest_event_preview(
    content: &matrix_sdk::store::SerializableEventContent,
) -> Option<String> {
    let content: matrix_sdk::ruma::events::AnyMessageLikeEventContent =
        content.deserialize().ok()?;
    match content {
        matrix_sdk::ruma::events::AnyMessageLikeEventContent::RoomMessage(message) => {
            Some(message.body().to_owned())
        }
        _ => None,
    }
}

async fn matrix_room_tags(room: &matrix_sdk::Room) -> MatrixRoomTags {
    let tags = room.tags().await.ok().flatten();
    let favourite = tags
        .as_ref()
        .and_then(|tags| tags.get(&matrix_sdk::ruma::events::tag::TagName::Favorite))
        .map(matrix_room_tag_info_from_sdk)
        .or_else(|| {
            room.is_favourite()
                .then_some(MatrixRoomTagInfo { order: None })
        });
    let low_priority = tags
        .as_ref()
        .and_then(|tags| tags.get(&matrix_sdk::ruma::events::tag::TagName::LowPriority))
        .map(matrix_room_tag_info_from_sdk)
        .or_else(|| {
            room.is_low_priority()
                .then_some(MatrixRoomTagInfo { order: None })
        });

    MatrixRoomTags {
        favourite,
        low_priority,
    }
}

fn matrix_room_tag_info_from_sdk(
    info: &matrix_sdk::ruma::events::tag::TagInfo,
) -> MatrixRoomTagInfo {
    MatrixRoomTagInfo {
        order: info.order.map(|order| order.to_string()),
    }
}

pub fn room_attention_summary_from_room(room: &matrix_sdk::Room) -> Option<RoomAttentionSummary> {
    let room_display_name = room.cached_display_name().map(|name| name.to_string())?;
    let unread_notifications = room.unread_notification_counts();

    room_attention_summary_from_counts(
        Some(room_display_name),
        room.is_dm(),
        unread_notifications.notification_count.into(),
        unread_notifications.highlight_count.into(),
        room.num_unread_messages(),
        room.is_marked_unread(),
    )
}

async fn matrix_parent_space_ids(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(parent_spaces) = room.parent_spaces().await else {
        return Vec::new();
    };

    parent_spaces
        .filter_map(|parent_space| async move {
            match parent_space.ok()? {
                ParentSpace::Reciprocal(space) | ParentSpace::WithPowerlevel(space) => {
                    Some(space.room_id().to_string())
                }
                ParentSpace::Illegitimate(_) | ParentSpace::Unverifiable(_) => None,
            }
        })
        .collect()
        .await
}

async fn matrix_space_child_room_ids(room: &matrix_sdk::Room) -> Vec<String> {
    let Ok(child_events) = room
        .get_state_events_static::<SpaceChildEventContent>()
        .await
    else {
        return Vec::new();
    };

    let mut child_room_ids: Vec<String> = child_events
        .into_iter()
        .filter_map(|child_event| match child_event.deserialize() {
            Ok(SyncOrStrippedState::Sync(SyncStateEvent::Original(event))) => {
                Some(event.state_key.to_string())
            }
            Ok(SyncOrStrippedState::Sync(SyncStateEvent::Redacted(_))) => None,
            Ok(SyncOrStrippedState::Stripped(event)) => Some(event.state_key.to_string()),
            Err(_) => None,
        })
        .collect();
    child_room_ids.sort();
    child_room_ids.dedup();
    child_room_ids
}

#[cfg(test)]
mod tests {
    use super::{
        MatrixConversationActivity, MatrixConversationActivitySource, MatrixRoomTagInfo,
        MatrixRoomTags, SdkUnreadTrace, SpaceMemberLookupStatus, classify_space_member_ids,
        matrix_conversation_activity_from_timeline_event, matrix_conversation_activity_source,
        matrix_room_latest_event_projection, matrix_room_list_room_from_counts,
        matrix_timeline_event_is_redacted, newest_conversation_activity,
        people_scope_diagnostic_event, space_members_scope_diagnostic_event,
        trace_sdk_unread_snapshot,
    };

    use koushi_diagnostics::DiagnosticValue;
    use std::collections::BTreeMap;
    #[test]
    fn people_scope_diagnostic_distinguishes_direct_space_members_from_child_room_aggregate() {
        let event = people_scope_diagnostic_event(true, 7, 3);

        assert_eq!(event.source, "sdk.people_scope");
        assert_eq!(event.stage, "member_snapshot");
        let field = |key| {
            event
                .fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| &field.value)
        };
        assert_eq!(
            field("source"),
            Some(&koushi_diagnostics::DiagnosticValue::Token(
                "direct_space_members"
            ))
        );
        assert_eq!(
            field("direct_member_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(7))
        );
        assert_eq!(
            field("child_room_count"),
            Some(&koushi_diagnostics::DiagnosticValue::Count(3))
        );
        assert_eq!(
            field("child_room_members_included"),
            Some(&koushi_diagnostics::DiagnosticValue::Boolean(false))
        );
    }
    #[test]
    fn conversation_activity_classifies_messages_encryption_and_threads_only() {
        use MatrixConversationActivitySource::{EncryptedMessage, Message, ThreadReply};

        let cases = [
            ("m.room.message", None, Some(Message)),
            ("m.room.message", Some("m.in_reply_to"), Some(Message)),
            ("m.room.message", Some("m.thread"), Some(ThreadReply)),
            ("m.room.encrypted", None, Some(EncryptedMessage)),
            ("m.room.encrypted", Some("m.thread"), Some(ThreadReply)),
            ("m.room.message", Some("m.replace"), None),
            ("m.room.message", Some("m.annotation"), None),
            ("m.room.redaction", None, None),
            ("m.room.member", None, None),
            ("m.room.name", None, None),
            ("m.reaction", Some("m.annotation"), None),
            ("m.receipt", None, None),
            ("m.typing", None, None),
            ("m.presence", None, None),
        ];

        for (event_type, relation_type, expected) in cases {
            assert_eq!(
                matrix_conversation_activity_source(event_type, relation_type),
                expected,
                "unexpected classification for {event_type} / {relation_type:?}"
            );
        }
    }
    #[test]
    fn redacted_raw_latest_is_classified_without_private_diagnostics() {
        let event = matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
            matrix_sdk::ruma::serde::Raw::from_json_string(
                serde_json::json!({
                    "content": {"body": "redacted body", "msgtype": "m.text"},
                    "event_id": "$redacted:example.invalid",
                    "origin_server_ts": 42,
                    "sender": "@sender:example.invalid",
                    "type": "m.room.message",
                    "unsigned": {"redacted_because": {"type": "m.room.redaction"}}
                })
                .to_string(),
            )
            .expect("redacted synthetic timeline event"),
        );

        assert!(matrix_timeline_event_is_redacted(&event));
    }

    #[test]
    fn malformed_unsigned_latest_fails_closed_as_redacted() {
        let event = matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
            matrix_sdk::ruma::serde::Raw::from_json_string(
                serde_json::json!({
                    "content": {"body": "private body", "msgtype": "m.text"},
                    "event_id": "$private:example.invalid",
                    "origin_server_ts": 42,
                    "sender": "@private:example.invalid",
                    "type": "m.room.message",
                    "unsigned": "malformed"
                })
                .to_string(),
            )
            .expect("malformed unsigned synthetic timeline event"),
        );

        assert!(matrix_timeline_event_is_redacted(&event));
        assert!(matrix_conversation_activity_from_timeline_event(&event).is_none());
    }

    #[test]
    fn redacted_cached_latest_does_not_create_conversation_activity() {
        let event = matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
            matrix_sdk::ruma::serde::Raw::from_json_string(
                serde_json::json!({
                    "content": {"body": "redacted body", "msgtype": "m.text"},
                    "event_id": "$redacted:example.invalid",
                    "origin_server_ts": 42,
                    "sender": "@sender:example.invalid",
                    "type": "m.room.message",
                    "unsigned": {"redacted_because": {"type": "m.room.redaction"}}
                })
                .to_string(),
            )
            .expect("redacted synthetic timeline event"),
        );

        assert!(matrix_conversation_activity_from_timeline_event(&event).is_none());
    }

    #[tokio::test]
    async fn remote_replacement_latest_keeps_original_identity_and_order() {
        use matrix_sdk::ruma::{event_id, room_id, user_id};
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

        let room_id = room_id!("!room:example.invalid");
        let sender = user_id!("@sender:example.invalid");
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        client
            .event_cache()
            .subscribe()
            .expect("event cache subscription");

        let original_factory = EventFactory::new().room(room_id).sender(sender);
        let replacement_factory = EventFactory::new().room(room_id).sender(sender);
        let original = original_factory
            .server_ts(42)
            .text_msg("before")
            .event_id(event_id!("$original:example.invalid"))
            .into_raw_sync();
        let replacement = replacement_factory
            .server_ts(99)
            .text_msg("fallback edit body")
            .event_id(event_id!("$edit:example.invalid"))
            .edit(
                event_id!("$original:example.invalid"),
                matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(
                    "after",
                )
                .into(),
            )
            .into_raw_sync();

        let room = server
            .sync_room(&client, JoinedRoomBuilder::new(room_id))
            .await;
        let mut latest_events = client
            .latest_events()
            .await
            .listen_and_subscribe_to_room(room_id)
            .await
            .expect("latest event subscription")
            .expect("latest event stream");
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id).add_timeline_bulk(vec![original, replacement]),
            )
            .await;
        tokio::time::timeout(std::time::Duration::from_secs(1), latest_events.next())
            .await
            .expect("latest event update must be event-driven");

        let (latest, activity) = matrix_room_latest_event_projection(&room).await;
        let latest = latest.expect("latest replacement projection");
        assert_eq!(latest.event_id, "$original:example.invalid");
        assert_eq!(latest.sender_id.as_deref(), Some("@sender:example.invalid"));
        assert_eq!(latest.timestamp_ms, 42);
        assert_eq!(latest.relation_type, None);
        assert_eq!(latest.relation_event_id, None);
        assert_eq!(latest.preview.as_deref(), Some("after"));
        assert_eq!(activity.map(|activity| activity.timestamp_ms), Some(42));
    }

    #[test]
    fn conversation_activity_keeps_the_newest_cache_or_local_candidate() {
        let cached = super::MatrixConversationActivity {
            timestamp_ms: 41,
            source: MatrixConversationActivitySource::EncryptedMessage,
        };
        let local = super::MatrixConversationActivity {
            timestamp_ms: 42,
            source: MatrixConversationActivitySource::Message,
        };

        assert_eq!(
            newest_conversation_activity(Some(cached), Some(local)),
            Some(local)
        );
        assert_eq!(
            newest_conversation_activity(Some(cached), None),
            Some(cached)
        );
        assert_eq!(newest_conversation_activity(None, None), None);
    }
    #[test]
    fn conversation_activity_debug_hides_raw_timestamp() {
        let activity = MatrixConversationActivity {
            timestamp_ms: 42,
            source: MatrixConversationActivitySource::ThreadReply,
        };

        let debug = format!("{activity:?}");

        assert!(debug.contains("ThreadReply"), "{debug}");
        assert!(!debug.contains("42"), "{debug}");
    }
    #[test]
    fn joined_room_list_prefers_async_direct_dm_detection() {
        let source = include_str!("room_projection.rs");
        let projection_body =
            crate::test_source::item_body(source, "async fn matrix_room_list_snapshot_from_rooms");

        assert!(
            projection_body.contains("room.is_direct().await"),
            "joined room projection should read m.direct via async Room::is_direct"
        );
        assert!(
            projection_body.contains("unwrap_or_else(|_| room.is_dm())"),
            "joined room projection should fall back to cached is_dm when direct lookup fails"
        );
    }
    #[test]
    fn joined_room_list_snapshot_avoids_full_member_scans() {
        let source = include_str!("room_projection.rs");
        let projection_body =
            crate::test_source::item_body(source, "async fn matrix_room_list_snapshot_from_rooms");

        assert!(projection_body.contains("room.joined_members_count()"));
        assert!(projection_body.contains("matrix_space_member_user_ids_no_sync(&room).await"));
        assert!(!projection_body.contains("collect_active_member_profiles"));
        assert!(
            !projection_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "room-list projection must not load the full active member list"
        );
        assert!(
            !projection_body.contains("joined_user_ids"),
            "room-list projection should not derive joined members from joined_user_ids"
        );
    }
    #[test]
    fn joined_room_list_dm_resolution_uses_account_data_cached_and_heroes_candidates() {
        let source = include_str!("room_projection.rs");
        let helper_body =
            crate::test_source::item_body(source, "async fn matrix_room_list_dm_user_ids");

        assert!(
            helper_body.contains("direct_targets_by_room.get(&room_id)"),
            "DM resolution should prefer direct account-data targets first"
        );
        assert!(
            helper_body.contains(".direct_targets()"),
            "DM resolution should fall back to cached SDK direct targets"
        );
        assert!(
            helper_body.contains("room.heroes()"),
            "DM resolution should use heroes when the room is already considered a DM"
        );
        assert!(
            helper_body.contains("get_member_no_sync"),
            "DM resolution should only probe candidate members without syncing the full list"
        );
        assert!(
            helper_body.contains("dm_user_ids.push(candidate_user_id_string.clone())"),
            "DM resolution should preserve valid direct/cached/hero IDs even when a local member profile is absent"
        );
        assert!(
            !helper_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "DM resolution helper must not hide a full active-member scan behind the hot path"
        );
        assert!(
            !helper_body.contains("room.members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)"),
            "DM resolution helper must not enumerate every active member even without syncing"
        );
    }
    #[test]
    fn space_member_ids_are_no_sync_and_space_only() {
        let source = include_str!("room_projection.rs");
        let helper_body =
            crate::test_source::item_body(source, "async fn matrix_space_member_user_ids_no_sync");

        assert!(
            helper_body.contains("members_no_sync(matrix_sdk::RoomMemberships::JOIN)"),
            "space membership may read only already-synced local state"
        );
        assert!(
            !helper_body.contains("RoomMemberships::ACTIVE"),
            "space membership helper must not flatten joined and invited users"
        );
        assert!(
            !helper_body.contains("room.members(matrix_sdk::RoomMemberships::JOIN)"),
            "space membership helper must not sync/fetch the full member list"
        );
    }
    #[test]
    fn space_member_facts_separate_join_invite_and_child_only() {
        let facts = classify_space_member_ids(
            ["joined", "both"],
            ["invited"],
            [
                ("child-a", ["child-only", "both"]),
                ("child-b", ["child-only", "second-only"]),
            ],
        );

        assert_eq!(facts.space_joined_ids, vec!["both", "joined"]);
        assert_eq!(facts.space_invited_ids, vec!["invited"]);
        assert_eq!(facts.child_room_only_ids, vec!["child-only", "second-only"]);
        assert_eq!(facts.child_join_union_count, 3);
        assert_eq!(facts.duplicate_child_membership_count, 1);
        assert_eq!(
            facts.child_room_ids.get("child-only"),
            Some(&vec!["child-a".to_owned(), "child-b".to_owned()])
        );
    }
    #[test]
    fn joined_only_helpers_do_not_use_active_membership() {
        let source = include_str!("room_projection.rs");
        let body =
            crate::test_source::item_body(source, "async fn matrix_space_members_projection");
        assert!(!body.contains("RoomMemberships::ACTIVE"));
        assert!(body.contains("members_no_sync(matrix_sdk::RoomMemberships::JOIN)"));
        assert!(body.contains("members_no_sync(matrix_sdk::RoomMemberships::INVITE)"));
    }
    #[test]
    fn space_lookup_failures_are_not_coerced_to_empty_observations() {
        let source = include_str!("room_projection.rs");
        let body =
            crate::test_source::item_body(source, "pub async fn matrix_space_members_projection");
        let joined_lookup = body
            .split("let space_joined_members = match")
            .nth(1)
            .expect("Space JOIN lookup exists")
            .split("let space_invited_members")
            .next()
            .expect("Space INVITE lookup follows JOIN lookup");
        let invited_lookup = body
            .split("let space_invited_members = match")
            .nth(1)
            .expect("Space INVITE lookup exists")
            .split("let mut space_joined_by_user")
            .next()
            .expect("Space member classification follows Space lookups");

        for lookup in [joined_lookup, invited_lookup] {
            assert!(
                lookup.contains("Err(error)"),
                "Space lookup errors must retain their structured error"
            );
            assert!(
                lookup.contains("return Err(MatrixRoomOperationError::from_sdk_error(error))"),
                "Space lookup errors must abort the projection instead of becoming empty input"
            );
        }
    }
    #[test]
    fn failed_space_member_counts_are_reported_as_unavailable() {
        let source = include_str!("room_projection.rs");
        let body = crate::test_source::item_body(source, "fn space_members_scope_diagnostic_event");

        assert!(body.contains("space_join_lookup_outcome"));
        assert!(body.contains("space_invite_lookup_outcome"));
        assert!(body.contains("counts_unavailable"));
        assert!(body.contains("space_joined_lookup.observed_count()"));
        assert!(body.contains("space_invited_lookup.observed_count()"));
        assert!(body.contains("if let Some(count)"));
    }
    #[test]
    fn space_members_scope_diagnostic_is_private_data_free() {
        let event = space_members_scope_diagnostic_event(
            "observed",
            SpaceMemberLookupStatus::Observed(2),
            SpaceMemberLookupStatus::Observed(1),
            Some(3),
            Some(2),
            Some(1),
            Some(2),
            Some(1),
            Some(4),
            Some(4),
            Some(1),
            Some(2),
            1,
            0,
        );

        assert_eq!(event.source, "sdk.space_members_scope");
        let rendered = format!("{event:?}");
        for forbidden in [
            "!space:example.invalid",
            "!child:example.invalid",
            "@alice:example.invalid",
            "Alice",
            "mxc://example.invalid/avatar",
            "raw sdk error",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "diagnostic leaked private data: {forbidden}"
            );
        }
    }
    #[test]
    fn failed_space_member_lookup_does_not_report_zero_counts() {
        let event = space_members_scope_diagnostic_event(
            "observed",
            SpaceMemberLookupStatus::Failed,
            SpaceMemberLookupStatus::NotAttempted,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            0,
            1,
        );

        assert!(event.fields.iter().any(|field| {
            field.key == "space_join_lookup_outcome"
                && field.value == DiagnosticValue::Token("lookup_failed")
        }));
        assert!(event.fields.iter().any(|field| {
            field.key == "space_join_count_availability"
                && field.value == DiagnosticValue::Token("counts_unavailable")
        }));
        assert!(event.fields.iter().any(|field| {
            field.key == "space_invite_lookup_outcome"
                && field.value == DiagnosticValue::Token("not_attempted")
        }));
        for field in &event.fields {
            if matches!(
                field.key,
                "space_joined_count"
                    | "space_invited_count"
                    | "child_room_count"
                    | "child_room_only_count"
                    | "input_count"
                    | "output_count"
            ) {
                assert_ne!(
                    field.value,
                    DiagnosticValue::Count(0),
                    "unobserved Space counts must not be fabricated as zero"
                );
            }
        }
    }
    #[test]
    fn matrix_room_member_summaries_still_scans_full_members() {
        let source = include_str!("room_projection.rs");
        let helper_body =
            crate::test_source::item_body(source, "async fn matrix_room_member_summaries");

        assert!(
            helper_body.contains("room.members(matrix_sdk::RoomMemberships::ACTIVE)"),
            "member summaries should still be allowed to load the full active member list"
        );
    }
    #[test]
    fn direct_account_data_targets_are_indexed_by_room() {
        use matrix_sdk::ruma::{
            OwnedRoomId, OwnedUserId,
            events::direct::{DirectEventContent, OwnedDirectUserIdentifier},
        };

        let alice: OwnedUserId = "@alice:example.invalid".try_into().unwrap();
        let bob: OwnedUserId = "@bob:example.invalid".try_into().unwrap();
        let dm_room: OwnedRoomId = "!dm:example.invalid".try_into().unwrap();
        let other_room: OwnedRoomId = "!other:example.invalid".try_into().unwrap();
        let mut content = DirectEventContent::default();
        content.insert(
            OwnedDirectUserIdentifier::from(alice),
            vec![dm_room.clone(), other_room.clone()],
        );
        content.insert(OwnedDirectUserIdentifier::from(bob), vec![dm_room.clone()]);

        let by_room = super::direct_account_data_targets_by_room(&content);

        assert_eq!(
            by_room.get(dm_room.as_str()),
            Some(&vec![
                "@alice:example.invalid".to_owned(),
                "@bob:example.invalid".to_owned()
            ])
        );
        assert_eq!(
            by_room.get(other_room.as_str()),
            Some(&vec!["@alice:example.invalid".to_owned()])
        );
    }
    #[test]
    fn direct_account_data_targets_are_sorted_deduplicated_and_indexed_by_room() {
        use matrix_sdk::ruma::{
            OwnedRoomId, OwnedUserId,
            events::direct::{DirectEventContent, OwnedDirectUserIdentifier},
        };

        let alice: OwnedUserId = "@alice:example.invalid".try_into().unwrap();
        let bob: OwnedUserId = "@bob:example.invalid".try_into().unwrap();
        let room: OwnedRoomId = "!dm:example.invalid".try_into().unwrap();
        let mut content = DirectEventContent::default();
        content.insert(OwnedDirectUserIdentifier::from(bob), vec![room.clone()]);
        content.insert(OwnedDirectUserIdentifier::from(alice), vec![room.clone()]);

        assert_eq!(
            super::direct_account_data_targets_by_room(&content),
            BTreeMap::from([(
                room.to_string(),
                vec![
                    "@alice:example.invalid".to_owned(),
                    "@bob:example.invalid".to_owned(),
                ],
            )]),
        );
    }
    #[test]
    fn live_direct_account_data_loader_is_local_only() {
        let source = include_str!("room_projection.rs");
        let body = crate::test_source::item_body(
            source,
            "pub async fn cached_direct_account_data_targets_by_room",
        );
        assert!(body.contains("account_data::<DirectEventContent>()"));
        assert!(!body.contains("fetch_account_data_static"));
    }
    #[tokio::test]
    async fn explicit_empty_direct_map_overrides_cached_room_direct_targets() {
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::JoinedRoomBuilder;
        use serde_json::json;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let target = matrix_sdk::ruma::user_id!("@dm-target:example.org");
        let room_id = matrix_sdk::ruma::room_id!("!stale-dm:example.org");

        server
            .mock_sync()
            .ok_and_run(&client, |builder| {
                builder.add_custom_global_account_data(json!({
                    "type": "m.direct",
                    "content": { target: [room_id] }
                }));
                builder.add_joined_room(JoinedRoomBuilder::new(room_id));
            })
            .await;

        let room = client.get_room(&room_id).expect("joined test room");
        assert!(
            !room.direct_targets().is_empty(),
            "test room must have cached direct targets"
        );

        let direct_targets_by_room = BTreeMap::new();
        let snapshot = super::room_list_snapshot_from_sdk_rooms_with_direct_targets(
            std::iter::once(room),
            Some(&direct_targets_by_room),
        )
        .await;

        assert_eq!(snapshot.rooms.len(), 1);
        assert!(!snapshot.rooms[0].is_dm);
    }
    #[test]
    fn direct_account_data_dm_detection_fetches_server_when_store_misses() {
        let source = include_str!("room_projection.rs");
        let helper_body = crate::test_source::item_body(
            source,
            "async fn matrix_direct_account_data_targets_by_room",
        );

        assert!(helper_body.contains("account_data::<DirectEventContent>()"));
        assert!(helper_body.contains("fetch_account_data_static::<DirectEventContent>()"));
    }
    #[test]
    fn room_list_room_from_counts_carries_notification_metadata() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            true,
            vec!["@alice:example.invalid".to_owned()],
            MatrixRoomTags::default(),
            4,
            2,
            2,
            false,
            None,
            None,
            None,
            None,
            None,
            vec!["!space:example.invalid".to_owned()],
            false,
            2,
        );

        assert_eq!(room.notification_count, 4);
        assert_eq!(room.highlight_count, 2);
        assert_eq!(room.unread_count, 2);
        assert_eq!(room.joined_members, 2);
        assert!(room.is_dm);
    }
    #[test]
    fn room_list_room_from_counts_does_not_turn_manual_unread_into_messages() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            MatrixRoomTags::default(),
            0,
            0,
            0,
            true,
            None,
            None,
            None,
            None,
            None,
            vec![],
            false,
            0,
        );

        assert_eq!(room.unread_count, 0);
        assert!(room.marked_unread);
    }
    #[test]
    fn room_list_room_from_counts_carries_room_tags() {
        let tags = MatrixRoomTags {
            favourite: Some(MatrixRoomTagInfo {
                order: Some("0.25".to_owned()),
            }),
            low_priority: None,
        };

        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            tags.clone(),
            0,
            0,
            0,
            false,
            None,
            None,
            None,
            None,
            None,
            vec![],
            false,
            0,
        );

        assert_eq!(room.tags, tags);
    }
    #[test]
    fn unread_diagnostic_snapshot_rejects_private_synthetic_inputs() {
        let latest_event = Some(crate::MatrixRoomLatestEventSummary {
            event_id: "$event:example.invalid".to_owned(),
            sender_id: Some("@user:example.invalid".to_owned()),
            sender_label: None,
            sender_avatar_mxc_uri: None,
            preview: Some("secret message".to_owned()),
            timestamp_ms: 42,
            event_type: Some("m.room.message".to_owned()),
            relation_type: None,
            relation_event_id: None,
            content_converted: true,
            is_threaded: false,
            is_reply: false,
            has_thread_summary: false,
            has_reactions: false,
            is_redacted: false,
        });
        trace_sdk_unread_snapshot(SdkUnreadTrace {
            unread_messages: 2,
            unread_count: 2,
            notification_count: 1,
            highlight_count: 1,
            marked_unread: true,
            latest_event: &latest_event,
            fully_read_event_id: Some("$event:example.invalid"),
            private_read_receipt_event_id: None,
            recency_stamp_present: true,
            conversation_activity: Some(MatrixConversationActivity {
                timestamp_ms: 3,
                source: MatrixConversationActivitySource::Message,
            }),
        });
        let serialized = serde_json::to_string(&koushi_diagnostics::snapshot()).unwrap();
        assert!(serialized.contains("conversation_activity_source"));
        for forbidden in [
            "!room:example.invalid",
            "@user:example.invalid",
            "$event:example.invalid",
            "/Users/alice/private",
            "secret message",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized diagnostics leaked {forbidden}"
            );
        }
        assert!(serialized.contains("unread_messages"));
        assert!(serialized.contains("latest_event_present"));
    }
    fn test_latest_event(event_id: &str) -> crate::MatrixRoomLatestEventSummary {
        crate::MatrixRoomLatestEventSummary {
            event_id: event_id.to_owned(),
            sender_id: None,
            sender_label: None,
            sender_avatar_mxc_uri: None,
            preview: None,
            timestamp_ms: 42,
            event_type: Some("m.room.message".to_owned()),
            relation_type: None,
            relation_event_id: None,
            content_converted: true,
            is_threaded: false,
            is_reply: false,
            has_thread_summary: false,
            has_reactions: false,
            is_redacted: false,
        }
    }
    #[test]
    fn room_list_room_from_counts_rejects_redacted_latest_as_read_marker_target() {
        let mut latest = test_latest_event("$latest:example.invalid");
        latest.is_redacted = true;
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            MatrixRoomTags::default(),
            2,
            1,
            2,
            false,
            Some(42),
            None,
            Some(latest),
            Some("$latest:example.invalid".to_owned()),
            None,
            vec![],
            false,
            2,
        );

        assert_eq!(room.unread_count, 2);
        assert_eq!(room.notification_count, 2);
        assert_eq!(room.highlight_count, 1);
    }

    #[test]
    fn room_list_room_from_counts_suppresses_stale_unread_when_fully_read_matches_latest_event() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            MatrixRoomTags::default(),
            2,
            1,
            2,
            false,
            Some(42),
            None,
            Some(test_latest_event("$latest:example.invalid")),
            Some("$latest:example.invalid".to_owned()),
            None,
            vec![],
            false,
            2,
        );

        assert_eq!(room.unread_count, 0);
        assert_eq!(room.notification_count, 0);
        assert_eq!(room.highlight_count, 0);
        assert!(!room.marked_unread);
    }
    #[test]
    fn room_list_room_from_counts_preserves_unread_when_read_marker_differs_from_latest_event() {
        let room = matrix_room_list_room_from_counts(
            "!room:example.invalid".to_owned(),
            "Room".to_owned(),
            None,
            false,
            Vec::new(),
            MatrixRoomTags::default(),
            2,
            1,
            2,
            false,
            Some(42),
            None,
            Some(test_latest_event("$latest:example.invalid")),
            Some("$older:example.invalid".to_owned()),
            None,
            vec![],
            false,
            2,
        );

        assert_eq!(room.unread_count, 2);
        assert_eq!(room.notification_count, 2);
        assert_eq!(room.highlight_count, 1);
    }
}
