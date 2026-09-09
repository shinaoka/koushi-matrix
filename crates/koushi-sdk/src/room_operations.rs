#[cfg(test)]
use crate::MatrixRoomMemberRoleOption;
use crate::room_projection::{
    matrix_public_room_from_chunk, matrix_room, matrix_room_operation_failure_kind,
    matrix_room_settings_snapshot, matrix_space_members_projection, non_empty_name,
    room_settings_snapshot_with_change, room_settings_snapshot_with_member_power_level,
    sdk_history_visibility, sdk_join_rule_for_update,
};
use crate::{
    MatrixClientSession, MatrixRoomMemberSummary, MatrixRoomTagKind, MatrixSpaceMembersProjection,
};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticLevel};
#[cfg(test)]
use koushi_state::SessionInfo;
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixRoomOperationError {
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix room alias is invalid")]
    InvalidRoomAlias,
    #[error("Matrix room setting is invalid")]
    InvalidRoomSetting,
    #[error("Matrix event id is invalid")]
    InvalidEventId,
    #[error("Matrix user id is invalid")]
    InvalidUserId,
    #[error("Matrix server name is invalid")]
    InvalidServerName,
    #[error("Matrix room is not available")]
    RoomUnavailable,
    #[error("Matrix room operation failed: {0}")]
    Sdk(MatrixRoomOperationFailureKind),
}

impl MatrixRoomOperationError {
    pub fn failure_kind(&self) -> Option<MatrixRoomOperationFailureKind> {
        match self {
            Self::Sdk(kind) => Some(*kind),
            Self::InvalidRoomId
            | Self::InvalidRoomAlias
            | Self::InvalidRoomSetting
            | Self::InvalidEventId
            | Self::InvalidUserId
            | Self::InvalidServerName
            | Self::RoomUnavailable => None,
        }
    }

    pub(super) fn from_sdk_error(error: matrix_sdk::Error) -> Self {
        Self::Sdk(matrix_room_operation_failure_kind(&error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomOperationFailureKind {
    AuthenticationRequired,
    Encryption,
    Forbidden,
    Http,
    Store,
    SecureBackupRequired,
    WrongRoomState,
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixSpaceInviteCancellationOutcome {
    Cancelled,
    NotInvited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixSpaceMemberRoleFailureKind {
    Forbidden,
    Stale,
    NotFound,
    Network,
    Timeout,
    Invalid,
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixSpaceMemberRoleUpdateResult {
    pub succeeded: bool,
    pub failure_kind: Option<MatrixSpaceMemberRoleFailureKind>,
    pub sent_revision: Option<String>,
    pub projection: Option<MatrixSpaceMembersProjection>,
}

impl fmt::Display for MatrixRoomOperationFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::AuthenticationRequired => "authentication_required",
            Self::Encryption => "encryption",
            Self::Forbidden => "forbidden",
            Self::Http => "http",
            Self::Store => "store",
            Self::SecureBackupRequired => "secure_backup_required",
            Self::WrongRoomState => "wrong_room_state",
            Self::Sdk => "sdk",
        };
        formatter.write_str(label)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryQuery {
    pub term: Option<String>,
    pub server_name: Option<String>,
    pub limit: Option<u32>,
    pub since: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryResult {
    pub rooms: Vec<MatrixPublicRoomDirectoryRoom>,
    pub next_batch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixPublicRoomDirectoryRoom {
    pub room_id: String,
    pub canonical_alias: Option<String>,
    /// Matrix `room_type`, e.g. `m.space`. Absent for an ordinary room.
    pub room_type: Option<String>,
    /// Empty when the directory entry has no name; the caller decides how to
    /// label it, because a room and a space need different fallbacks.
    pub name: String,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub joined_members: u64,
    pub world_readable: bool,
    pub guest_can_join: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixCreateRoomOptions {
    pub name: String,
    pub topic: Option<String>,
    pub alias_localpart: Option<String>,
    pub encrypted: bool,
    pub visibility: MatrixCreateRoomVisibility,
    pub parent_space: Option<MatrixCreateRoomParentSpace>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixCreateRoomVisibility {
    Private,
    Public,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixCreateRoomParentSpace {
    pub space_id: String,
    pub via_server: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomSettingsSnapshot {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub canonical_alias: Option<String>,
    pub alternate_aliases: Vec<String>,
    pub share_link: Option<String>,
    pub join_rule: MatrixRoomJoinRule,
    pub history_visibility: MatrixRoomHistoryVisibility,
    pub permissions: MatrixRoomPermissionFacts,
    pub members: Vec<MatrixRoomMemberSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixRoomMemberRole {
    Creator,
    Administrator,
    Moderator,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixUserTrustState {
    Unverified,
    Verified,
    IdentityReset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomJoinRule {
    Public,
    Invite,
    Knock,
    Restricted,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomHistoryVisibility {
    WorldReadable,
    Shared,
    Invited,
    Joined,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixRoomPermissionFacts {
    pub can_edit_settings: bool,
    pub can_edit_roles: bool,
    pub can_invite: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_unban: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixRoomSettingChange {
    Name(Option<String>),
    Topic(Option<String>),
    AvatarUrl(Option<String>),
    JoinRule(MatrixRoomJoinRule),
    HistoryVisibility(MatrixRoomHistoryVisibility),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomModerationAction {
    Kick,
    Ban,
    Unban,
}

pub async fn room_can_send_text_message(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    if room.state() != matrix_sdk::RoomState::Joined {
        return Ok(false);
    }

    let power_levels = room.power_levels_or_default().await;
    Ok(power_levels.user_can_send_message(
        room.own_user_id(),
        matrix_sdk::ruma::events::MessageLikeEventType::RoomMessage,
    ))
}

pub async fn get_room_settings_snapshot(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    Ok(matrix_room_settings_snapshot(&room).await)
}

pub async fn update_room_setting(
    session: &MatrixClientSession,
    room_id: &str,
    change: MatrixRoomSettingChange,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let snapshot = matrix_room_settings_snapshot(&room).await;
    match &change {
        MatrixRoomSettingChange::Name(name) => {
            room.set_name(name.clone().unwrap_or_default())
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::Topic(topic) => {
            room.set_room_topic(topic.as_deref().unwrap_or_default())
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::AvatarUrl(Some(avatar_url)) => {
            let avatar_url = matrix_sdk::ruma::OwnedMxcUri::from(avatar_url.clone());
            room.set_avatar_url(avatar_url.as_ref(), None)
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::AvatarUrl(None) => {
            room.remove_avatar()
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::JoinRule(join_rule) => {
            let join_rule = sdk_join_rule_for_update(*join_rule)?;
            room.privacy_settings()
                .update_join_rule(join_rule)
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
        MatrixRoomSettingChange::HistoryVisibility(history_visibility) => {
            room.privacy_settings()
                .update_room_history_visibility(sdk_history_visibility(*history_visibility))
                .await
                .map_err(MatrixRoomOperationError::from_sdk_error)?;
        }
    }

    Ok(room_settings_snapshot_with_change(snapshot, &change))
}

pub async fn moderate_room_member(
    session: &MatrixClientSession,
    room_id: &str,
    target_user_id: &str,
    action: MatrixRoomModerationAction,
    reason: Option<&str>,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;

    match action {
        MatrixRoomModerationAction::Kick => room.kick_user(&target_user_id, reason).await,
        MatrixRoomModerationAction::Ban => room.ban_user(&target_user_id, reason).await,
        MatrixRoomModerationAction::Unban => room.unban_user(&target_user_id, reason).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

struct RawSpacePowerLevels {
    revision: Option<String>,
    content: matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
}

async fn raw_space_power_levels(
    room: &matrix_sdk::Room,
) -> Result<RawSpacePowerLevels, MatrixRoomOperationError> {
    let event = room
        .get_state_event_static::<
            matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
        >()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    let Some(event) = event else {
        return Ok(RawSpacePowerLevels {
            revision: None,
            content: matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent::new(
                &matrix_sdk::ruma::room_version_rules::AuthorizationRules::V1,
            ),
        });
    };
    let state = event
        .deserialize()
        .map_err(|_| MatrixRoomOperationError::InvalidRoomSetting)?;
    let revision = state.event_id().map(ToString::to_string);
    let content = state
        .original_content()
        .cloned()
        .ok_or(MatrixRoomOperationError::InvalidRoomSetting)?;
    Ok(RawSpacePowerLevels { revision, content })
}

fn effective_space_power_level(
    content: &matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
    user_id: &matrix_sdk::ruma::UserId,
) -> i64 {
    content
        .users
        .get(user_id)
        .copied()
        .unwrap_or(content.users_default)
        .into()
}

fn unrelated_power_levels_equal(
    left: &matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
    right: &matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
    target_user_id: &matrix_sdk::ruma::UserId,
) -> bool {
    left.ban == right.ban
        && left.events == right.events
        && left.events_default == right.events_default
        && left.invite == right.invite
        && left.kick == right.kick
        && left.redact == right.redact
        && left.state_default == right.state_default
        && left.users_default == right.users_default
        && left.notifications.room == right.notifications.room
        && left
            .users
            .iter()
            .filter(|(user_id, _)| *user_id != target_user_id)
            .eq(right
                .users
                .iter()
                .filter(|(user_id, _)| *user_id != target_user_id))
}

fn set_space_power_level(
    content: &mut matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
    target_user_id: matrix_sdk::ruma::OwnedUserId,
    power_level: i64,
) -> Result<(), MatrixRoomOperationError> {
    let power_level = matrix_sdk::ruma::Int::try_from(power_level)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomSetting)?;
    if power_level == content.users_default {
        content.users.remove(&target_user_id);
    } else {
        content.users.insert(target_user_id, power_level);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpaceRoleConvergenceDecision {
    Waiting,
    Succeeded,
    Failed(MatrixSpaceMemberRoleFailureKind),
}

fn classify_space_role_observation(
    observed_revision: Option<&str>,
    expected_revision: Option<&str>,
    sent_revision: &str,
    target_matches: bool,
    unrelated_matches: bool,
) -> SpaceRoleConvergenceDecision {
    if observed_revision == expected_revision {
        SpaceRoleConvergenceDecision::Waiting
    } else if observed_revision == Some(sent_revision) {
        if target_matches && unrelated_matches {
            SpaceRoleConvergenceDecision::Succeeded
        } else {
            SpaceRoleConvergenceDecision::Failed(MatrixSpaceMemberRoleFailureKind::Stale)
        }
    } else if observed_revision.is_some() {
        SpaceRoleConvergenceDecision::Failed(MatrixSpaceMemberRoleFailureKind::Stale)
    } else {
        SpaceRoleConvergenceDecision::Waiting
    }
}

fn rederive_space_role_outcome(
    proven_success: bool,
    provisional_failure: MatrixSpaceMemberRoleFailureKind,
    latest_revision: Option<&str>,
    expected_revision: Option<&str>,
    sent_revision: &str,
    latest_target_matches: bool,
) -> SpaceRoleConvergenceDecision {
    let latest_revision_is_authoritative = latest_revision == Some(sent_revision)
        || (latest_revision.is_some() && latest_revision != expected_revision);
    if (proven_success || latest_revision_is_authoritative) && latest_target_matches {
        SpaceRoleConvergenceDecision::Succeeded
    } else if latest_target_matches {
        SpaceRoleConvergenceDecision::Failed(provisional_failure)
    } else {
        SpaceRoleConvergenceDecision::Failed(MatrixSpaceMemberRoleFailureKind::Stale)
    }
}

async fn send_space_power_levels(
    room: &matrix_sdk::Room,
    content: matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent,
) -> Result<String, MatrixRoomOperationError> {
    room.send_state_event(content)
        .await
        .map(|response| response.event_id.to_string())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

fn role_update_result(
    succeeded: bool,
    failure_kind: Option<MatrixSpaceMemberRoleFailureKind>,
    sent_revision: Option<String>,
    projection: Option<MatrixSpaceMembersProjection>,
) -> MatrixSpaceMemberRoleUpdateResult {
    MatrixSpaceMemberRoleUpdateResult {
        succeeded,
        failure_kind,
        sent_revision,
        projection,
    }
}

fn projection_target_power_level(
    projection: &MatrixSpaceMembersProjection,
    target_user_id: &str,
) -> Option<i64> {
    projection
        .space_joined
        .iter()
        .find(|entry| entry.user_id == target_user_id)
        .and_then(|entry| entry.power_level)
}

fn projection_target_has_option(
    projection: &MatrixSpaceMembersProjection,
    target_user_id: &str,
    power_level: i64,
    confirmed: bool,
) -> bool {
    projection
        .space_joined
        .iter()
        .find(|entry| entry.user_id == target_user_id)
        .and_then(|entry| {
            entry
                .role_options
                .iter()
                .find(|option| option.power_level == power_level)
        })
        .is_some_and(|option| !option.requires_confirmation || confirmed)
}

const SPACE_MEMBER_ROLE_UPDATE_DEADLINE: Duration = Duration::from_secs(10);

/// Update one directly-joined Space member with a revision-scoped, event-driven
/// convergence wait. The returned projection is always a fresh SDK projection;
/// no local role patch is returned.
pub async fn update_space_member_power_level(
    session: &MatrixClientSession,
    space_id: &str,
    target_user_id: &str,
    expected_power_levels_revision: Option<&str>,
    expected_power_level: i64,
    power_level: i64,
    confirmed: bool,
) -> Result<MatrixSpaceMemberRoleUpdateResult, MatrixRoomOperationError> {
    let room = matrix_room(session, space_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let initial_projection = matrix_space_members_projection(session, space_id).await?;
    if initial_projection.power_levels_revision.as_deref() != expected_power_levels_revision {
        return Ok(role_update_result(
            false,
            Some(MatrixSpaceMemberRoleFailureKind::Stale),
            None,
            Some(initial_projection),
        ));
    }
    if !initial_projection.can_edit_roles {
        return Ok(role_update_result(
            false,
            Some(MatrixSpaceMemberRoleFailureKind::Forbidden),
            None,
            Some(initial_projection),
        ));
    }
    if projection_target_power_level(&initial_projection, target_user_id.as_str())
        != Some(expected_power_level)
    {
        return Ok(role_update_result(
            false,
            Some(MatrixSpaceMemberRoleFailureKind::Stale),
            None,
            Some(initial_projection),
        ));
    }
    if !projection_target_has_option(
        &initial_projection,
        target_user_id.as_str(),
        power_level,
        confirmed,
    ) {
        return Ok(role_update_result(
            false,
            Some(MatrixSpaceMemberRoleFailureKind::Invalid),
            None,
            Some(initial_projection),
        ));
    }

    // Subscribe before the final preflight: a cached immediate read is not a
    // terminal observation and may still show expected_revision.
    let mut updates = room.subscribe_to_updates();
    let final_raw = raw_space_power_levels(&room).await?;
    let final_projection = matrix_space_members_projection(session, space_id).await?;
    if final_raw.revision.as_deref() != expected_power_levels_revision
        || final_projection.power_levels_revision.as_deref() != expected_power_levels_revision
        || projection_target_power_level(&final_projection, target_user_id.as_str())
            != Some(expected_power_level)
    {
        return Ok(role_update_result(
            false,
            Some(MatrixSpaceMemberRoleFailureKind::Stale),
            None,
            Some(final_projection),
        ));
    }
    if !final_projection.can_edit_roles
        || !projection_target_has_option(
            &final_projection,
            target_user_id.as_str(),
            power_level,
            confirmed,
        )
    {
        return Ok(role_update_result(
            false,
            Some(MatrixSpaceMemberRoleFailureKind::Forbidden),
            None,
            Some(final_projection),
        ));
    }

    let mut send_content = final_raw.content.clone();
    set_space_power_level(&mut send_content, target_user_id.to_owned(), power_level)?;
    let sent_revision = send_space_power_levels(&room, send_content.clone()).await?;

    let mut proven_success = false;
    let mut provisional_failure = MatrixSpaceMemberRoleFailureKind::Timeout;
    let deadline = tokio::time::Instant::now() + SPACE_MEMBER_ROLE_UPDATE_DEADLINE;
    loop {
        let observed = raw_space_power_levels(&room).await?;
        let target_matches =
            effective_space_power_level(&observed.content, &target_user_id) == power_level;
        let unrelated_matches =
            unrelated_power_levels_equal(&observed.content, &send_content, &target_user_id);
        match classify_space_role_observation(
            observed.revision.as_deref(),
            expected_power_levels_revision,
            &sent_revision,
            target_matches,
            unrelated_matches,
        ) {
            SpaceRoleConvergenceDecision::Waiting => {}
            SpaceRoleConvergenceDecision::Succeeded => {
                proven_success = true;
                break;
            }
            SpaceRoleConvergenceDecision::Failed(kind) => {
                provisional_failure = kind;
                break;
            }
        }

        match tokio::time::timeout_at(deadline, updates.recv()).await {
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                provisional_failure = MatrixSpaceMemberRoleFailureKind::Network;
                break;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                provisional_failure = MatrixSpaceMemberRoleFailureKind::Network;
                break;
            }
            Err(_) => {
                provisional_failure = MatrixSpaceMemberRoleFailureKind::Timeout;
                break;
            }
        }
    }

    let latest_raw = raw_space_power_levels(&room).await?;
    let latest_projection = matrix_space_members_projection(session, space_id).await?;
    let latest_target = projection_target_power_level(&latest_projection, target_user_id.as_str());
    let latest_target_matches = latest_target == Some(power_level)
        && latest_projection.power_levels_revision == latest_raw.revision
        && latest_raw
            .content
            .users
            .get(&target_user_id)
            .copied()
            .unwrap_or(latest_raw.content.users_default)
            == matrix_sdk::ruma::Int::try_from(power_level)
                .map_err(|_| MatrixRoomOperationError::InvalidRoomSetting)?;

    match rederive_space_role_outcome(
        proven_success,
        provisional_failure,
        latest_raw.revision.as_deref(),
        expected_power_levels_revision,
        &sent_revision,
        latest_target_matches,
    ) {
        SpaceRoleConvergenceDecision::Succeeded => Ok(role_update_result(
            true,
            None,
            Some(sent_revision),
            Some(latest_projection),
        )),
        SpaceRoleConvergenceDecision::Failed(kind) => Ok(role_update_result(
            false,
            Some(kind),
            Some(sent_revision),
            Some(latest_projection),
        )),
        SpaceRoleConvergenceDecision::Waiting => Ok(role_update_result(
            false,
            Some(provisional_failure),
            Some(sent_revision),
            Some(latest_projection),
        )),
    }
}

pub async fn update_room_member_power_level(
    session: &MatrixClientSession,
    room_id: &str,
    target_user_id: &str,
    power_level: i64,
) -> Result<MatrixRoomSettingsSnapshot, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let target_user_id = matrix_sdk::ruma::UserId::parse(target_user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let power_level = matrix_sdk::ruma::Int::try_from(power_level)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomSetting)?;

    room.update_power_levels(vec![(target_user_id.as_ref(), power_level)])
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;

    let target_user_id_ref: &matrix_sdk::ruma::UserId = target_user_id.as_ref();
    Ok(room_settings_snapshot_with_member_power_level(
        matrix_room_settings_snapshot(&room).await,
        target_user_id_ref.as_str(),
        power_level.into(),
    ))
}

pub async fn create_room(
    session: &MatrixClientSession,
    options: MatrixCreateRoomOptions,
) -> Result<String, MatrixRoomOperationError> {
    if matches!(options.visibility, MatrixCreateRoomVisibility::Public)
        && preview_room_address(
            &options.name,
            Some(options.alias_localpart.as_deref().unwrap_or("")),
            session.client().user_id().map(|id| id.as_str()),
        )
        .error
        .is_some()
    {
        return Err(MatrixRoomOperationError::InvalidRoomAlias);
    }
    let request = create_room_request(options)?;
    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn create_public_directory_room(
    session: &MatrixClientSession,
    name: &str,
    alias_localpart: &str,
) -> Result<String, MatrixRoomOperationError> {
    create_room(
        session,
        MatrixCreateRoomOptions {
            name: name.to_owned(),
            topic: None,
            alias_localpart: Some(alias_localpart.to_owned()),
            encrypted: false,
            visibility: MatrixCreateRoomVisibility::Public,
            parent_space: None,
        },
    )
    .await
}

pub(super) fn create_room_request(
    options: MatrixCreateRoomOptions,
) -> Result<matrix_sdk::ruma::api::client::room::create_room::v3::Request, MatrixRoomOperationError>
{
    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(&options.name);
    request.topic = options
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|topic| !topic.is_empty())
        .map(ToOwned::to_owned);

    let is_public = matches!(options.visibility, MatrixCreateRoomVisibility::Public);
    if is_public {
        let alias_localpart = options
            .alias_localpart
            .as_deref()
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .ok_or(MatrixRoomOperationError::InvalidRoomAlias)?;
        validate_alias_localpart(alias_localpart)?;
        request.room_alias_name = Some(alias_localpart.to_owned());
        request.visibility = matrix_sdk::ruma::api::client::room::Visibility::Public;
        request.preset =
            Some(matrix_sdk::ruma::api::client::room::create_room::v3::RoomPreset::PublicChat);
    }

    if options.encrypted && !is_public {
        request.initial_state.push(
            matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent::with_recommended_defaults(),
            )
            .to_raw_any(),
        );
    }

    if let Some(parent_space) = options.parent_space {
        let parent_space_id = matrix_sdk::ruma::OwnedRoomId::try_from(parent_space.space_id)
            .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
        let via_server = matrix_sdk::ruma::OwnedServerName::try_from(parent_space.via_server)
            .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;
        let mut parent_content =
            matrix_sdk::ruma::events::space::parent::SpaceParentEventContent::new(vec![via_server]);
        parent_content.canonical = true;
        request.initial_state.push(
            matrix_sdk::ruma::events::InitialStateEvent::new(
                parent_space_id.clone(),
                parent_content,
            )
            .to_raw_any(),
        );

        if !is_public {
            request.room_version = Some(matrix_sdk::ruma::RoomVersionId::V9);
            request.initial_state.push(
                matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                    matrix_sdk::ruma::events::room::join_rules::RoomJoinRulesEventContent::restricted(
                        vec![
                            matrix_sdk::ruma::events::room::join_rules::AllowRule::room_membership(
                                parent_space_id,
                            ),
                        ],
                    ),
                )
                .to_raw_any(),
            );
            request.initial_state.push(
                matrix_sdk::ruma::events::InitialStateEvent::with_empty_state_key(
                    matrix_sdk::ruma::events::room::history_visibility::RoomHistoryVisibilityEventContent::new(
                        matrix_sdk::ruma::events::room::history_visibility::HistoryVisibility::Invited,
                    ),
                )
                .to_raw_any(),
            );
        }
    }

    Ok(request)
}

/// Resolve a draft against the account's Matrix server; this never probes availability.
pub fn preview_room_address(
    name: &str,
    alias_localpart: Option<&str>,
    user_id: Option<&str>,
) -> koushi_state::RoomAddressPreview {
    use koushi_state::{RoomAddressError, RoomAddressPreview, suggest_room_alias_localpart};
    let localpart = alias_localpart
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| suggest_room_alias_localpart(name));
    let user = user_id.and_then(|id| matrix_sdk::ruma::UserId::parse(id).ok());
    let error = if user.is_none() {
        Some(RoomAddressError::NotReady)
    } else if localpart.is_empty() {
        Some(RoomAddressError::Empty)
    } else if validate_alias_localpart(&localpart).is_err() {
        Some(RoomAddressError::Invalid)
    } else {
        None
    };
    let mut preview = RoomAddressPreview {
        localpart,
        full_alias: None,
        error,
    };
    if preview.error.is_none() {
        let alias = format!(
            "#{}:{}",
            preview.localpart,
            user.expect("validated user").server_name()
        );
        // The SDK enables Ruma's arbitrary-length compatibility feature; new
        // aliases still obey Matrix's 255-byte identifier limit.
        if alias.len() > 255 {
            preview.error = Some(RoomAddressError::Invalid);
            return preview;
        }
        match matrix_sdk::ruma::RoomAliasId::parse(alias) {
            Ok(alias) => preview.full_alias = Some(alias.to_string()),
            Err(_) => preview.error = Some(RoomAddressError::Invalid),
        }
    }
    preview
}

fn validate_alias_localpart(alias_localpart: &str) -> Result<(), MatrixRoomOperationError> {
    if alias_localpart.starts_with('#')
        || alias_localpart.contains(':')
        || alias_localpart
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(MatrixRoomOperationError::InvalidRoomAlias);
    }
    Ok(())
}

pub async fn create_space(
    session: &MatrixClientSession,
    name: &str,
) -> Result<String, MatrixRoomOperationError> {
    let mut creation_content =
        matrix_sdk::ruma::api::client::room::create_room::v3::CreationContent::new();
    creation_content.room_type = Some(matrix_sdk::ruma::room::RoomType::Space);

    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.name = non_empty_name(name);
    request.creation_content = Some(
        matrix_sdk::ruma::serde::Raw::new(&creation_content)
            .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))?,
    );

    let room = session
        .client()
        .create_room(request)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn invite_user_to_room(
    session: &MatrixClientSession,
    room_id: &str,
    user_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    room.invite_user_by_id(&user_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn cancel_space_invite(
    session: &MatrixClientSession,
    space_id: &str,
    user_id: &str,
) -> Result<MatrixSpaceInviteCancellationOutcome, MatrixRoomOperationError> {
    let room = matrix_room(session, space_id)?;
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let invited_members = room
        .members_no_sync(matrix_sdk::RoomMemberships::INVITE)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    if !invited_members
        .iter()
        .any(|member| member.user_id().as_str() == user_id.as_str())
    {
        return Ok(MatrixSpaceInviteCancellationOutcome::NotInvited);
    }
    room.kick_user(&user_id, None)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(MatrixSpaceInviteCancellationOutcome::Cancelled)
}

pub async fn room_has_active_member_no_sync(
    session: &MatrixClientSession,
    room_id: &str,
    user_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    let members = room
        .members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(members
        .iter()
        .any(|member| member.user_id().as_str() == user_id.as_str()))
}

pub async fn start_direct_message(
    session: &MatrixClientSession,
    user_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "sdk.room_operation",
        "start_dm_started",
    ));
    let user_id = matrix_sdk::ruma::UserId::parse(user_id)
        .map_err(|_| MatrixRoomOperationError::InvalidUserId)?;
    // Get-or-create (#368): reuse the existing joined DM whose only direct
    // target is this user. Unconditional create_dm minted a duplicate DM room
    // on every repeated "Send message".
    if let Some(room) = session.client().get_dm_room(&user_id) {
        koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "sdk.room_operation",
            "start_dm_reused",
        ));
        return Ok(room.room_id().to_string());
    }
    koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "sdk.room_operation",
        "start_dm_create_started",
    ));
    let room = match session.client().create_dm(&user_id).await {
        Ok(room) => room,
        Err(error) => {
            koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "sdk.room_operation",
                "start_dm_create_failed",
            ));
            return Err(MatrixRoomOperationError::from_sdk_error(error));
        }
    };
    koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "sdk.room_operation",
        "start_dm_create_completed",
    ));
    Ok(room.room_id().to_string())
}

#[cfg(test)]
mod start_direct_message_tests {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::JoinedRoomBuilder;
    use serde_json::json;

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
        MatrixClientSession {
            client,
            info,
            diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
        }
    }

    #[tokio::test]
    async fn start_direct_message_reuses_the_existing_joined_dm_room() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let target = matrix_sdk::ruma::user_id!("@dm-target:example.org");
        let dm_room_id = matrix_sdk::ruma::room_id!("!existing-dm:example.org");

        // Seed a joined room marked as the DM with the target through
        // m.direct. No createRoom endpoint is mounted, so an accidental
        // create_dm would fail the call instead of silently passing.
        server
            .mock_sync()
            .ok_and_run(&session.client(), |builder| {
                builder.add_custom_global_account_data(json!({
                    "type": "m.direct",
                    "content": { target: [dm_room_id] }
                }));
                builder.add_joined_room(JoinedRoomBuilder::new(dm_room_id));
            })
            .await;

        let started = super::start_direct_message(&session, target.as_str())
            .await
            .expect("existing DM must be reused without a create call");
        assert_eq!(started, dm_room_id.to_string());
    }

    #[tokio::test]
    async fn start_direct_message_creates_a_room_only_when_no_dm_exists() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let target = matrix_sdk::ruma::user_id!("@fresh-target:example.org");

        server.mock_create_room().ok().mock_once().mount().await;

        let started = super::start_direct_message(&session, target.as_str())
            .await
            .expect("a missing DM must be created");
        // The prebuilt createRoom mock answers with this fixed room id.
        assert_eq!(started, "!room:example.org");
    }
}

pub async fn join_room_by_id(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "sdk.room_operation",
        "join_started",
    ));
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let room = match session.client().join_room_by_id(&room_id).await {
        Ok(room) => room,
        Err(error) => {
            koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "sdk.room_operation",
                "join_failed",
            ));
            return Err(MatrixRoomOperationError::from_sdk_error(error));
        }
    };
    koushi_diagnostics::record_and_stderr(DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "sdk.room_operation",
        "join_completed",
    ));
    Ok(room.room_id().to_string())
}

pub async fn query_public_room_directory(
    session: &MatrixClientSession,
    query: MatrixPublicRoomDirectoryQuery,
) -> Result<MatrixPublicRoomDirectoryResult, MatrixRoomOperationError> {
    let mut filter = matrix_sdk::ruma::directory::Filter::new();
    filter.generic_search_term = query.term;

    let mut request =
        matrix_sdk::ruma::api::client::directory::get_public_rooms_filtered::v3::Request::new();
    request.filter = filter;
    request.limit = query.limit.map(Into::into);
    request.since = query.since;
    request.server = query
        .server_name
        .map(matrix_sdk::ruma::OwnedServerName::try_from)
        .transpose()
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;

    let response = session
        .client()
        .public_rooms_filtered(request)
        .await
        .map_err(|error| MatrixRoomOperationError::from_sdk_error(error.into()))?;

    Ok(MatrixPublicRoomDirectoryResult {
        rooms: response
            .chunk
            .into_iter()
            .map(matrix_public_room_from_chunk)
            .collect(),
        next_batch: response.next_batch,
    })
}

/// Coarse joinability of a previewed room, for deciding what to offer.
///
/// The exact join rule is server policy; the GUI only needs to know whether a
/// plain Join is expected to work, so restricted/knock variants collapse here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixPreviewJoinability {
    /// Anyone may join.
    Open,
    /// An invite (or a knock) is required first.
    InviteOnly,
    /// Joining depends on membership of another room.
    Restricted,
    /// The server did not report a join rule.
    Unknown,
}

/// Membership the current account already has in a previewed room.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixPreviewMembership {
    Joined,
    Invited,
    /// Not a member, or the room is unknown to this account.
    None,
}

/// A private-data-minimized preview of a room the user has not joined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomPreview {
    pub room_id: String,
    pub canonical_alias: Option<String>,
    /// Matrix `room_type`, e.g. `m.space`. Absent for an ordinary room.
    pub room_type: Option<String>,
    /// Empty when the room has no name; the caller supplies the fallback.
    pub name: String,
    pub topic: Option<String>,
    pub joined_members: u64,
    pub joinability: MatrixPreviewJoinability,
    pub membership: MatrixPreviewMembership,
}

/// Project an SDK room preview into the private-data-minimized DTO.
pub(super) fn matrix_room_preview_from_sdk(
    preview: matrix_sdk::room_preview::RoomPreview,
) -> MatrixRoomPreview {
    use matrix_sdk::ruma::room::JoinRuleSummary;

    let joinability = match preview.join_rule {
        Some(JoinRuleSummary::Public) => MatrixPreviewJoinability::Open,
        Some(JoinRuleSummary::Invite | JoinRuleSummary::Knock | JoinRuleSummary::Private) => {
            MatrixPreviewJoinability::InviteOnly
        }
        Some(JoinRuleSummary::Restricted(_) | JoinRuleSummary::KnockRestricted(_)) => {
            MatrixPreviewJoinability::Restricted
        }
        _ => MatrixPreviewJoinability::Unknown,
    };
    let membership = match preview.state {
        Some(matrix_sdk::RoomState::Joined) => MatrixPreviewMembership::Joined,
        Some(matrix_sdk::RoomState::Invited) => MatrixPreviewMembership::Invited,
        _ => MatrixPreviewMembership::None,
    };
    MatrixRoomPreview {
        room_id: preview.room_id.to_string(),
        canonical_alias: preview.canonical_alias.map(|alias| alias.to_string()),
        room_type: preview.room_type.map(|room_type| room_type.to_string()),
        name: preview.name.unwrap_or_default(),
        topic: preview.topic,
        joined_members: preview.num_joined_members,
        joinability,
        membership,
    }
}

pub async fn preview_join_target(
    session: &MatrixClientSession,
    target: &MatrixJoinTarget,
) -> Result<MatrixRoomPreview, MatrixRoomOperationError> {
    let (room_or_alias, via) = resolve_join_target(target)?;
    let preview = session
        .client()
        .get_room_preview(room_or_alias.as_ref(), via)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(matrix_room_preview_from_sdk(preview))
}

/// A room to join, as named by a directory result or a `matrix.to` link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatrixJoinTarget {
    /// `#alias:server` or `!id:server`.
    pub room_id_or_alias: String,
    /// Servers to try when the homeserver does not already know the room.
    pub via_servers: Vec<String>,
}

/// Resolve a join target into the ids the SDK join call needs.
///
/// A room id is a legitimate join target, not a malformed alias: links carry
/// ids far more often than aliases. Every `via` server is kept, because for an
/// id target they are the only way the homeserver can locate a room it has
/// never seen - dropping all but the first silently breaks federated joins.
pub(super) fn resolve_join_target(
    target: &MatrixJoinTarget,
) -> Result<
    (
        matrix_sdk::ruma::OwnedRoomOrAliasId,
        Vec<matrix_sdk::ruma::OwnedServerName>,
    ),
    MatrixRoomOperationError,
> {
    let room_or_alias = matrix_sdk::ruma::RoomOrAliasId::parse(&target.room_id_or_alias)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomAlias)?;
    let via = target
        .via_servers
        .iter()
        .map(|server| {
            matrix_sdk::ruma::OwnedServerName::try_from(server.as_str())
                .map_err(|_| MatrixRoomOperationError::InvalidServerName)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((room_or_alias, via))
}

pub async fn join_room_target(
    session: &MatrixClientSession,
    target: &MatrixJoinTarget,
) -> Result<String, MatrixRoomOperationError> {
    let (room_or_alias, via) = resolve_join_target(target)?;
    let room = session
        .client()
        .join_room_by_id_or_alias(room_or_alias.as_ref(), &via)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room.room_id().to_string())
}

pub async fn leave_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let room_id = room.room_id().to_string();
    room.leave()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room_id)
}

pub async fn forget_room(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<String, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let room_id = room.room_id().to_string();
    room.forget()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(room_id)
}

pub async fn set_space_child(
    session: &MatrixClientSession,
    space_id: &str,
    child_room_id: &str,
    via_server: &str,
) -> Result<(), MatrixRoomOperationError> {
    let space = matrix_room(session, space_id)?;
    let child_room_id = matrix_sdk::ruma::OwnedRoomId::try_from(child_room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let via_server = matrix_sdk::ruma::OwnedServerName::try_from(via_server)
        .map_err(|_| MatrixRoomOperationError::InvalidServerName)?;
    let content =
        matrix_sdk::ruma::events::space::child::SpaceChildEventContent::new(vec![via_server]);

    space
        .send_state_event_for_key(&child_room_id, content)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub fn room_id_server_name(room_id: &str) -> Result<String, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    room_id
        .server_name()
        .map(ToString::to_string)
        .ok_or(MatrixRoomOperationError::InvalidRoomId)
}

pub async fn set_room_tag(
    session: &MatrixClientSession,
    room_id: &str,
    tag: MatrixRoomTagKind,
    order: Option<f64>,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    match tag {
        MatrixRoomTagKind::Favourite => room.set_is_favourite(true, order).await,
        MatrixRoomTagKind::LowPriority => room.set_is_low_priority(true, order).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn remove_room_tag(
    session: &MatrixClientSession,
    room_id: &str,
    tag: MatrixRoomTagKind,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    match tag {
        MatrixRoomTagKind::Favourite => room.set_is_favourite(false, None).await,
        MatrixRoomTagKind::LowPriority => room.set_is_low_priority(false, None).await,
    }
    .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn pin_event(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    room.pin_event(&event_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn unpin_event(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    room.unpin_event(&event_id)
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn mark_room_as_read(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    let receipts = matrix_sdk::room::Receipts::new()
        .fully_read_marker(event_id.clone())
        .private_read_receipt(event_id);
    room.send_multiple_receipts(receipts)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn mark_room_as_unread(
    session: &MatrixClientSession,
    room_id: &str,
    unread: bool,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.set_unread_flag(unread)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Discards the current outbound Megolm key so the next ordinary send rotates
/// through the SDK's standard create-and-share path.
pub async fn discard_outbound_room_key(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    if !room
        .latest_encryption_state()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?
        .is_encrypted()
    {
        return Err(MatrixRoomOperationError::Sdk(
            MatrixRoomOperationFailureKind::WrongRoomState,
        ));
    }
    room.discard_room_key()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

const ROOM_NOTIFICATION_RULE_ID_PREFIX: &str = "org.matrix.desktop.notify.room.";

fn room_notification_rule_id(room_id: &matrix_sdk::ruma::RoomId) -> String {
    format!("{ROOM_NOTIFICATION_RULE_ID_PREFIX}{room_id}")
}

/// Sets the per-room notification mode by manipulating app-owned push rules.
///
/// - `All`: removes any app-owned override/underride rule for the room.
/// - `Mentions`: adds an underride rule with empty actions so generic message
///   rules are suppressed but mention/highlight rules still fire.
/// - `Mute`: adds an override rule with empty actions so all notifications for
///   the room are suppressed.
pub async fn set_room_notification_mode(
    session: &MatrixClientSession,
    room_id: &str,
    mode: koushi_state::RoomNotificationMode,
) -> Result<(), MatrixRoomOperationError> {
    use matrix_sdk::ruma::{
        RoomId,
        api::client::push::{delete_pushrule, set_pushrule},
        push::{
            EventMatchConditionData, NewConditionalPushRule, NewPushRule, PushCondition, RuleKind,
        },
    };

    let room_id = RoomId::parse(room_id).map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    let rule_id = room_notification_rule_id(&room_id);
    let client = session.client();

    // Remove any previous app-owned rule for this room. Missing-rule errors are
    // ignored so the operation is idempotent.
    for kind in [RuleKind::Override, RuleKind::Underride] {
        let delete_request = delete_pushrule::v3::Request::new(kind, rule_id.clone());
        let _ = client.send(delete_request).await;
    }

    if mode != koushi_state::RoomNotificationMode::All {
        let actions = Vec::new();
        let conditions = vec![PushCondition::EventMatch(EventMatchConditionData::new(
            "room_id".to_owned(),
            room_id.to_string(),
        ))];
        let new_rule = NewConditionalPushRule::new(rule_id, conditions, actions);
        let new_push_rule = match mode {
            koushi_state::RoomNotificationMode::Mentions => NewPushRule::Underride(new_rule),
            koushi_state::RoomNotificationMode::Mute => NewPushRule::Override(new_rule),
            koushi_state::RoomNotificationMode::All => unreachable!(),
        };
        let set_request = set_pushrule::v3::Request::new(new_push_rule);
        client.send(set_request).await.map_err(|error| {
            MatrixRoomOperationError::from_sdk_error(matrix_sdk::Error::Http(Box::new(error)))
        })?;
    }

    Ok(())
}

pub async fn load_pinned_event_ids(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<Vec<String>, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let pinned = room
        .load_pinned_events()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?
        .unwrap_or_default();
    Ok(pinned
        .into_iter()
        .map(|event_id| event_id.to_string())
        .collect())
}

/// Whether the room's current membership is joined (issue #538: the actor
/// re-checks this when settling a manual encryption-debug completion, so a
/// completion that lands after the user left the room fails closed).
pub async fn room_is_joined(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    Ok(room.state() == matrix_sdk_base::RoomState::Joined)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod space_member_role_convergence_tests {
    use super::{
        MatrixClientSession, MatrixSpaceMemberRoleFailureKind, SpaceRoleConvergenceDecision,
        classify_space_role_observation, rederive_space_role_outcome, send_space_power_levels,
    };
    use matrix_sdk::{
        ruma::{
            event_id,
            events::{StateEventType, room::power_levels::RoomPowerLevelsEventContent},
            room_id,
            room_version_rules::AuthorizationRules,
        },
        test_utils::mocks::MatrixMockServer,
    };
    async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        let info = koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("mock user").to_string(),
            device_id: client.device_id().expect("mock device").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        MatrixClientSession {
            client,
            info,
            diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
        }
    }

    #[test]
    fn convergence_classifier_covers_expected_sent_unrelated_and_unknown_revisions() {
        let waiting = SpaceRoleConvergenceDecision::Waiting;
        let stale = SpaceRoleConvergenceDecision::Failed(MatrixSpaceMemberRoleFailureKind::Stale);
        assert_eq!(
            classify_space_role_observation(
                Some("$expected"),
                Some("$expected"),
                "$sent",
                false,
                false
            ),
            waiting
        );
        assert_eq!(
            classify_space_role_observation(Some("$sent"), Some("$expected"), "$sent", true, true),
            SpaceRoleConvergenceDecision::Succeeded
        );
        assert_eq!(
            classify_space_role_observation(Some("$sent"), Some("$expected"), "$sent", true, false),
            stale
        );
        assert_eq!(
            classify_space_role_observation(Some("$sent"), Some("$expected"), "$sent", false, true),
            stale
        );
        assert_eq!(
            classify_space_role_observation(Some("$other"), Some("$expected"), "$sent", true, true),
            stale
        );
        assert_eq!(
            classify_space_role_observation(None, Some("$expected"), "$sent", false, false),
            waiting
        );
    }

    #[test]
    fn final_fetch_rederivation_covers_success_upgrade_and_closed_failures() {
        let stale = MatrixSpaceMemberRoleFailureKind::Stale;
        let network = MatrixSpaceMemberRoleFailureKind::Network;
        assert_eq!(
            rederive_space_role_outcome(
                true,
                stale,
                Some("$later"),
                Some("$expected"),
                "$sent",
                true
            ),
            SpaceRoleConvergenceDecision::Succeeded
        );
        assert_eq!(
            rederive_space_role_outcome(
                false,
                network,
                Some("$sent"),
                Some("$expected"),
                "$sent",
                true
            ),
            SpaceRoleConvergenceDecision::Succeeded
        );
        assert_eq!(
            rederive_space_role_outcome(
                false,
                network,
                Some("$later"),
                Some("$expected"),
                "$sent",
                true
            ),
            SpaceRoleConvergenceDecision::Succeeded
        );
        assert_eq!(
            rederive_space_role_outcome(
                false,
                network,
                Some("$expected"),
                Some("$expected"),
                "$sent",
                true
            ),
            SpaceRoleConvergenceDecision::Failed(network)
        );
        assert_eq!(
            rederive_space_role_outcome(
                false,
                network,
                Some("$sent"),
                Some("$expected"),
                "$sent",
                false
            ),
            SpaceRoleConvergenceDecision::Failed(stale)
        );
        assert_eq!(
            rederive_space_role_outcome(
                false,
                MatrixSpaceMemberRoleFailureKind::Timeout,
                None,
                Some("$expected"),
                "$sent",
                true
            ),
            SpaceRoleConvergenceDecision::Failed(MatrixSpaceMemberRoleFailureKind::Timeout)
        );
    }

    #[tokio::test]
    async fn send_space_power_levels_returns_the_mocked_event_id() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let room_id = room_id!("!role-send:example.org");
        let room = server.sync_joined_room(&session.client(), room_id).await;
        let event_id = event_id!("$role-send:example.org");
        server
            .mock_room_send_state()
            .for_type(StateEventType::RoomPowerLevels)
            .ok(event_id)
            .expect(1)
            .mount()
            .await;

        let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V1);
        content.events_default = 7.into();
        let sent = send_space_power_levels(&room, content)
            .await
            .expect("mocked state event send");
        assert_eq!(sent, event_id.to_string());
    }
}
