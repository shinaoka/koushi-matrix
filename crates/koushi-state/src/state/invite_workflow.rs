use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    AppState, AvatarImage, OperationFailureKind, RoomManagementState, RoomMemberSummary,
    SpaceSummary, UserProfile,
};

pub const INVITE_ALREADY_IN_SPACE_MESSAGE: &str = "既にスペースにいます";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteWorkflowState {
    pub query: InviteTargetQueryState,
    #[serde(default)]
    pub selected_targets: Vec<InviteSelectedTarget>,
    #[serde(default)]
    pub scope_plan: Option<InviteScopePlan>,
    #[serde(default)]
    pub operation: InviteOperationState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteTargetQueryState {
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub candidates: Vec<InviteTargetCandidate>,
    #[serde(default)]
    pub explicit_user_id: Option<InviteTargetCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteTargetCandidate {
    pub user_id: String,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub source: InviteTargetCandidateSource,
    pub status: InviteTargetCandidateStatus,
    #[serde(default)]
    pub status_message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InviteTargetCandidateSource {
    Profile,
    LocalAlias,
    RoomMember,
    DirectMessage,
    MatrixId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InviteTargetCandidateStatus {
    Selectable,
    AlreadySelected,
    AlreadyInDestination,
    InvalidMatrixId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteSelectedTarget {
    pub user_id: String,
    pub display_label: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteScopePlan {
    pub room_id: String,
    pub destination_kind: InviteDestinationKind,
    pub default_scope: InviteScopeSelection,
    pub options: Vec<InviteScopeOption>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InviteDestinationKind {
    Room,
    Space,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InviteScopeSelection {
    RoomOnly,
    ParentSpaceAndRoom { space_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteScopeOption {
    pub scope: InviteScopeSelection,
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InviteOperationState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        room_id: String,
        user_ids: Vec<String>,
        scope: InviteScopeSelection,
    },
    Completed {
        request_id: u64,
        room_id: String,
        results: Vec<InviteDestinationResult>,
        #[serde(default)]
        notice: Option<String>,
    },
    Failed {
        request_id: u64,
        room_id: String,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InviteDestinationResult {
    pub user_id: String,
    pub destination: InviteDestination,
    pub kind: InviteDestinationResultKind,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InviteDestination {
    Room { room_id: String },
    Space { space_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InviteDestinationResultKind {
    Invited,
    AlreadyInSpace,
    Failed,
}

pub fn build_invite_target_query_state(
    state: &AppState,
    room_id: String,
    query: String,
) -> InviteTargetQueryState {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return InviteTargetQueryState {
            room_id: Some(room_id),
            query,
            candidates: Vec::new(),
            explicit_user_id: None,
        };
    }

    let lowered_query = trimmed.to_ascii_lowercase();
    let selected_user_ids = state
        .invite_workflow
        .selected_targets
        .iter()
        .map(|target| target.user_id.as_str())
        .collect::<BTreeSet<_>>();
    let destination_members = loaded_members_for_room(&state.room_management, &room_id)
        .map(|members| {
            members
                .iter()
                .map(|member| member.user_id.as_str())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    let mut candidates_by_user_id = BTreeMap::<String, InviteTargetCandidate>::new();

    for (user_id, profile) in &state.profile.users {
        if profile_matches_query(
            user_id,
            profile,
            state.profile.local_aliases.get(user_id).map(String::as_str),
            &lowered_query,
        ) {
            candidates_by_user_id.insert(
                user_id.clone(),
                candidate_from_profile(
                    user_id,
                    profile,
                    state.profile.local_aliases.get(user_id).map(String::as_str),
                    InviteTargetCandidateSource::Profile,
                    &selected_user_ids,
                    &destination_members,
                ),
            );
        }
    }

    for (user_id, alias) in &state.profile.local_aliases {
        if text_matches_query(alias, &lowered_query) || text_matches_query(user_id, &lowered_query)
        {
            candidates_by_user_id
                .entry(user_id.clone())
                .or_insert_with(|| {
                    candidate_from_parts(
                        user_id,
                        alias,
                        alias,
                        None,
                        InviteTargetCandidateSource::LocalAlias,
                        &selected_user_ids,
                        &destination_members,
                    )
                });
        }
    }

    for member in loaded_members_for_room(&state.room_management, &room_id).unwrap_or(&[]) {
        let alias = state
            .profile
            .local_aliases
            .get(&member.user_id)
            .map(String::as_str);
        if member_matches_query(member, alias, &lowered_query) {
            candidates_by_user_id
                .entry(member.user_id.clone())
                .or_insert_with(|| {
                    candidate_from_parts(
                        &member.user_id,
                        alias.unwrap_or(&member.display_label),
                        member
                            .original_display_label
                            .is_empty()
                            .then_some(member.display_label.as_str())
                            .unwrap_or(member.original_display_label.as_str()),
                        None,
                        InviteTargetCandidateSource::RoomMember,
                        &selected_user_ids,
                        &destination_members,
                    )
                });
        }
    }

    for room in &state.rooms {
        for user_id in &room.dm_user_ids {
            if text_matches_query(user_id, &lowered_query)
                || state.profile.users.get(user_id).is_some_and(|profile| {
                    profile_matches_query(
                        user_id,
                        profile,
                        state.profile.local_aliases.get(user_id).map(String::as_str),
                        &lowered_query,
                    )
                })
            {
                candidates_by_user_id
                    .entry(user_id.clone())
                    .or_insert_with(|| {
                        candidate_from_optional_profile(
                            user_id,
                            state.profile.users.get(user_id),
                            state.profile.local_aliases.get(user_id).map(String::as_str),
                            InviteTargetCandidateSource::DirectMessage,
                            &selected_user_ids,
                            &destination_members,
                        )
                    });
            }
        }
    }

    let explicit_user_id = if trimmed.starts_with('@') {
        let status = if is_valid_matrix_user_id(trimmed) {
            candidate_status(trimmed, &selected_user_ids, &destination_members)
        } else {
            InviteTargetCandidateStatus::InvalidMatrixId
        };
        Some(
            candidate_from_parts(
                trimmed,
                trimmed,
                trimmed,
                None,
                InviteTargetCandidateSource::MatrixId,
                &BTreeSet::new(),
                &BTreeSet::new(),
            )
            .with_status(status),
        )
    } else {
        None
    };

    let mut candidates = candidates_by_user_id.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.display_label
            .to_ascii_lowercase()
            .cmp(&right.display_label.to_ascii_lowercase())
            .then_with(|| left.user_id.cmp(&right.user_id))
    });
    candidates.truncate(8);

    InviteTargetQueryState {
        room_id: Some(room_id),
        query,
        candidates,
        explicit_user_id,
    }
}

pub fn build_invite_scope_plan(state: &AppState, room_id: String) -> InviteScopePlan {
    if state.spaces.iter().any(|space| space.space_id == room_id) {
        return InviteScopePlan {
            room_id,
            destination_kind: InviteDestinationKind::Space,
            default_scope: InviteScopeSelection::RoomOnly,
            options: vec![InviteScopeOption {
                scope: InviteScopeSelection::RoomOnly,
                label: "Space only".to_owned(),
                detail: None,
            }],
        };
    }

    let parent_space_ids = state
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .map(|room| room.parent_space_ids.clone())
        .unwrap_or_default();
    let mut ordered_parent_space_ids = Vec::new();
    if let Some(active_space_id) = &state.navigation.active_space_id {
        if parent_space_ids
            .iter()
            .any(|space_id| space_id == active_space_id)
        {
            ordered_parent_space_ids.push(active_space_id.clone());
        }
    }
    for space_id in parent_space_ids {
        if !ordered_parent_space_ids
            .iter()
            .any(|known| known == &space_id)
        {
            ordered_parent_space_ids.push(space_id);
        }
    }

    let mut options = Vec::new();
    for space_id in ordered_parent_space_ids {
        let label = space_label(&state.spaces, &space_id)
            .map(|name| format!("{name} and room"))
            .unwrap_or_else(|| "Parent space and room".to_owned());
        options.push(InviteScopeOption {
            scope: InviteScopeSelection::ParentSpaceAndRoom { space_id },
            label,
            detail: Some("Invite to the parent space before inviting to this room".to_owned()),
        });
    }
    options.push(InviteScopeOption {
        scope: InviteScopeSelection::RoomOnly,
        label: "Room only".to_owned(),
        detail: None,
    });
    let default_scope = options
        .first()
        .map(|option| option.scope.clone())
        .unwrap_or(InviteScopeSelection::RoomOnly);

    InviteScopePlan {
        room_id,
        destination_kind: InviteDestinationKind::Room,
        default_scope,
        options,
    }
}

pub fn invite_notice_from_results(results: &[InviteDestinationResult]) -> Option<String> {
    results.iter().find_map(|result| {
        (result.kind == InviteDestinationResultKind::AlreadyInSpace).then(|| {
            result
                .message
                .clone()
                .unwrap_or_else(|| INVITE_ALREADY_IN_SPACE_MESSAGE.to_owned())
        })
    })
}

pub fn selected_target_from_query(
    state: &InviteWorkflowState,
    user_id: &str,
) -> Option<InviteSelectedTarget> {
    state
        .query
        .candidates
        .iter()
        .find(|candidate| candidate.user_id == user_id)
        .or_else(|| {
            state
                .query
                .explicit_user_id
                .as_ref()
                .filter(|candidate| candidate.user_id == user_id)
        })
        .filter(|candidate| candidate.status == InviteTargetCandidateStatus::Selectable)
        .map(|candidate| InviteSelectedTarget {
            user_id: candidate.user_id.clone(),
            display_label: candidate.display_label.clone(),
            avatar: candidate.avatar.clone(),
        })
}

fn loaded_members_for_room<'a>(
    room_management: &'a RoomManagementState,
    room_id: &str,
) -> Option<&'a [RoomMemberSummary]> {
    room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .map(|settings| settings.members.as_slice())
}

fn profile_matches_query(
    user_id: &str,
    profile: &UserProfile,
    alias: Option<&str>,
    lowered_query: &str,
) -> bool {
    alias.is_some_and(|value| text_matches_query(value, lowered_query))
        || text_matches_query(user_id, lowered_query)
        || profile
            .display_name
            .as_deref()
            .is_some_and(|value| text_matches_query(value, lowered_query))
        || text_matches_query(&profile.display_label, lowered_query)
        || profile
            .mention_search_terms
            .iter()
            .any(|term| text_matches_query(term, lowered_query))
}

fn member_matches_query(
    member: &RoomMemberSummary,
    alias: Option<&str>,
    lowered_query: &str,
) -> bool {
    alias.is_some_and(|value| text_matches_query(value, lowered_query))
        || text_matches_query(&member.user_id, lowered_query)
        || member
            .display_name
            .as_deref()
            .is_some_and(|value| text_matches_query(value, lowered_query))
        || text_matches_query(&member.display_label, lowered_query)
        || text_matches_query(&member.original_display_label, lowered_query)
}

fn text_matches_query(value: &str, lowered_query: &str) -> bool {
    value.to_ascii_lowercase().contains(lowered_query)
}

fn candidate_from_profile(
    user_id: &str,
    profile: &UserProfile,
    alias: Option<&str>,
    source: InviteTargetCandidateSource,
    selected_user_ids: &BTreeSet<&str>,
    destination_members: &BTreeSet<&str>,
) -> InviteTargetCandidate {
    candidate_from_parts(
        user_id,
        alias.unwrap_or_else(|| non_empty(&profile.display_label).unwrap_or(user_id)),
        non_empty(&profile.original_display_label)
            .unwrap_or_else(|| non_empty(&profile.display_label).unwrap_or(user_id)),
        profile.avatar.clone(),
        source,
        selected_user_ids,
        destination_members,
    )
}

fn candidate_from_optional_profile(
    user_id: &str,
    profile: Option<&UserProfile>,
    alias: Option<&str>,
    source: InviteTargetCandidateSource,
    selected_user_ids: &BTreeSet<&str>,
    destination_members: &BTreeSet<&str>,
) -> InviteTargetCandidate {
    match profile {
        Some(profile) => candidate_from_profile(
            user_id,
            profile,
            alias,
            source,
            selected_user_ids,
            destination_members,
        ),
        None => candidate_from_parts(
            user_id,
            alias.unwrap_or(user_id),
            alias.unwrap_or(user_id),
            None,
            source,
            selected_user_ids,
            destination_members,
        ),
    }
}

fn candidate_from_parts(
    user_id: &str,
    display_label: &str,
    original_display_label: &str,
    avatar: Option<AvatarImage>,
    source: InviteTargetCandidateSource,
    selected_user_ids: &BTreeSet<&str>,
    destination_members: &BTreeSet<&str>,
) -> InviteTargetCandidate {
    InviteTargetCandidate {
        user_id: user_id.to_owned(),
        display_label: display_label.to_owned(),
        original_display_label: original_display_label.to_owned(),
        avatar,
        source,
        status: candidate_status(user_id, selected_user_ids, destination_members),
        status_message: None,
    }
}

fn candidate_status(
    user_id: &str,
    selected_user_ids: &BTreeSet<&str>,
    destination_members: &BTreeSet<&str>,
) -> InviteTargetCandidateStatus {
    if selected_user_ids.contains(user_id) {
        InviteTargetCandidateStatus::AlreadySelected
    } else if destination_members.contains(user_id) {
        InviteTargetCandidateStatus::AlreadyInDestination
    } else {
        InviteTargetCandidateStatus::Selectable
    }
}

trait InviteTargetCandidateExt {
    fn with_status(self, status: InviteTargetCandidateStatus) -> Self;
}

impl InviteTargetCandidateExt for InviteTargetCandidate {
    fn with_status(mut self, status: InviteTargetCandidateStatus) -> Self {
        self.status = status;
        self
    }
}

fn is_valid_matrix_user_id(value: &str) -> bool {
    let Some((localpart, server_name)) = value
        .strip_prefix('@')
        .and_then(|rest| rest.split_once(':'))
    else {
        return false;
    };
    !localpart.is_empty()
        && !server_name.is_empty()
        && !localpart.chars().any(char::is_whitespace)
        && !server_name.chars().any(char::is_whitespace)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn space_label<'a>(spaces: &'a [SpaceSummary], space_id: &str) -> Option<&'a str> {
    spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .and_then(|space| non_empty(&space.display_name))
}
