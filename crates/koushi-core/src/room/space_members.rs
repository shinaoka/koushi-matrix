use super::actor::{RoomActor, RoomMessage};
use super::normalization::avatar_from_mxc_uri;
use super::operations::{classify_room_error, operation_failure_kind};
use crate::executor;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::event::{CoreEvent, RoomEvent};
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_sdk::{
    MatrixClientSession, MatrixRoomOperationError, MatrixSpaceMemberEntry,
    MatrixSpaceMembersProjection,
};
use koushi_state::{
    AppAction, OperationFailureKind, SpaceMemberEntry, SpaceMemberInviteOutcome,
    SpaceMemberMembership, SpaceMembersProjection, UserProfile,
};
#[cfg(test)]
use koushi_state::{ProfileResolutionInput, ProfileResolutionSource, resolve_people_label};
use std::collections::{BTreeMap, BTreeSet};

const SPACE_MEMBER_REFRESH_CONNECTION_ID: RuntimeConnectionId = RuntimeConnectionId(0);

#[derive(Clone)]
pub(super) struct SpaceMemberDemand {
    space_id: String,
    generation: u64,
    child_room_ids: BTreeSet<String>,
    demand_generation: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct SpaceMemberRefreshFence {
    request_id: RequestId,
    session_generation: u64,
    demand_generation: u64,
    refresh_generation: u64,
}

fn state_space_members_projection(
    projection: MatrixSpaceMembersProjection,
    generation: u64,
) -> SpaceMembersProjection {
    SpaceMembersProjection {
        space_id: projection.space_id,
        generation,
        space_joined: projection
            .space_joined
            .into_iter()
            .map(|entry| state_space_member_entry(entry, SpaceMemberMembership::SpaceJoined))
            .collect(),
        space_invited: projection
            .space_invited
            .into_iter()
            .map(|entry| state_space_member_entry(entry, SpaceMemberMembership::SpaceInvited))
            .collect(),
        child_room_only: projection
            .child_room_only
            .into_iter()
            .map(|entry| state_space_member_entry(entry, SpaceMemberMembership::ChildRoomOnly))
            .collect(),
        child_room_count: projection.child_room_count,
        complete_child_room_count: projection.complete_child_room_count,
        incomplete_child_room_count: projection.incomplete_child_room_count,
        power_levels_revision: projection.power_levels_revision,
        can_edit_roles: projection.can_edit_roles,
    }
}

fn state_space_member_entry(
    entry: MatrixSpaceMemberEntry,
    membership: SpaceMemberMembership,
) -> SpaceMemberEntry {
    let display_name = entry
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let display_label = display_name
        .clone()
        .unwrap_or_else(|| "Unknown user".to_owned());
    SpaceMemberEntry {
        user_id: entry.user_id,
        display_name,
        display_label: display_label.clone(),
        original_display_label: display_label,
        avatar_url: entry.avatar_url,
        power_level: entry.power_level,
        role: match entry.role {
            koushi_sdk::MatrixRoomMemberRole::Creator => koushi_state::RoomMemberRole::Creator,
            koushi_sdk::MatrixRoomMemberRole::Administrator => {
                koushi_state::RoomMemberRole::Administrator
            }
            koushi_sdk::MatrixRoomMemberRole::Moderator => koushi_state::RoomMemberRole::Moderator,
            koushi_sdk::MatrixRoomMemberRole::User => koushi_state::RoomMemberRole::User,
        },
        membership,
        child_room_ids: entry.child_room_ids,
        invite_pending: false,
        role_options: entry
            .role_options
            .into_iter()
            .map(|option| koushi_state::SpaceMemberRoleOption {
                power_level: option.power_level,
                role: match option.role {
                    koushi_sdk::MatrixRoomMemberRole::Creator => {
                        koushi_state::RoomMemberRole::Creator
                    }
                    koushi_sdk::MatrixRoomMemberRole::Administrator => {
                        koushi_state::RoomMemberRole::Administrator
                    }
                    koushi_sdk::MatrixRoomMemberRole::Moderator => {
                        koushi_state::RoomMemberRole::Moderator
                    }
                    koushi_sdk::MatrixRoomMemberRole::User => koushi_state::RoomMemberRole::User,
                },
                requires_confirmation: option.requires_confirmation,
            })
            .collect(),
    }
}

/// Feed non-empty room observations into the account-scoped profile cache.
/// This is deliberately emitted alongside the Space projection, before the
/// projection action is reduced, so receipt/Seen payloads with no label can
/// resolve from `ProfileState.users` without requiring Space membership.
fn user_profiles_from_space_projection(
    projection: &MatrixSpaceMembersProjection,
) -> Vec<UserProfile> {
    let mut profiles = BTreeMap::<String, UserProfile>::new();
    for entry in projection
        .space_joined
        .iter()
        .chain(projection.space_invited.iter())
        .chain(projection.child_room_only.iter())
        .chain(projection.child_room_profiles.iter())
    {
        let has_display_name = entry
            .display_name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if !has_display_name && entry.avatar_url.is_none() {
            continue;
        }
        let next = UserProfile {
            user_id: entry.user_id.clone(),
            display_name: entry
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            display_label: entry
                .display_name
                .clone()
                .unwrap_or_else(|| "Unknown user".to_owned()),
            original_display_label: entry
                .display_name
                .clone()
                .unwrap_or_else(|| "Unknown user".to_owned()),
            mention_search_terms: Vec::new(),
            avatar: avatar_from_mxc_uri(entry.avatar_url.as_deref()),
        };
        profiles
            .entry(entry.user_id.clone())
            .and_modify(|existing| {
                if existing.display_name.is_none() && next.display_name.is_some() {
                    existing.display_name = next.display_name.clone();
                    existing.display_label = next.display_label.clone();
                    existing.original_display_label = next.original_display_label.clone();
                }
                if existing.avatar.is_none() && next.avatar.is_some() {
                    existing.avatar = next.avatar.clone();
                }
            })
            .or_insert(next);
    }
    profiles.into_values().collect()
}

struct SpaceInviteReconciliation {
    projection: SpaceMembersProjection,
    profiles: Vec<UserProfile>,
    outcome: SpaceMemberInviteOutcome,
}

struct SpaceInviteProjectionReconciliation {
    projection: SpaceMembersProjection,
    profiles: Vec<UserProfile>,
}

async fn reconcile_space_invite_outcome(
    session: &MatrixClientSession,
    space_id: &str,
    user_id: &str,
    generation: u64,
    fallback: SpaceMemberInviteOutcome,
) -> Option<SpaceInviteReconciliation> {
    let Ok(raw_projection) = koushi_sdk::matrix_space_members_projection(session, space_id).await
    else {
        return None;
    };
    let outcome = if raw_projection
        .space_joined
        .iter()
        .any(|entry| entry.user_id == user_id)
    {
        SpaceMemberInviteOutcome::AlreadyJoined
    } else if raw_projection
        .space_invited
        .iter()
        .any(|entry| entry.user_id == user_id)
    {
        SpaceMemberInviteOutcome::AlreadyInvited
    } else {
        fallback
    };
    let profiles = user_profiles_from_space_projection(&raw_projection);
    let projection = state_space_members_projection(raw_projection, generation);
    Some(SpaceInviteReconciliation {
        projection,
        profiles,
        outcome,
    })
}

async fn reconcile_space_invite_cancellation(
    session: &MatrixClientSession,
    space_id: &str,
    generation: u64,
) -> Option<SpaceInviteProjectionReconciliation> {
    let Ok(raw_projection) = koushi_sdk::matrix_space_members_projection(session, space_id).await
    else {
        return None;
    };
    let profiles = user_profiles_from_space_projection(&raw_projection);
    let projection = state_space_members_projection(raw_projection, generation);
    Some(SpaceInviteProjectionReconciliation {
        projection,
        profiles,
    })
}

#[cfg(test)]
fn record_core_space_members_projection(
    trigger: &'static str,
    generation: u64,
    projection: &SpaceMembersProjection,
    outcome: &'static str,
) {
    record_core_space_members_projection_with_metrics(
        trigger, generation, projection, None, outcome,
    );
}

fn record_core_space_members_projection_with_raw(
    trigger: &'static str,
    generation: u64,
    raw_projection: &MatrixSpaceMembersProjection,
    projection: &SpaceMembersProjection,
    outcome: &'static str,
) {
    record_core_space_members_projection_with_metrics(
        trigger,
        generation,
        projection,
        Some(raw_projection),
        outcome,
    );
}

fn record_core_space_members_projection_with_metrics(
    trigger: &'static str,
    generation: u64,
    projection: &SpaceMembersProjection,
    raw_projection: Option<&MatrixSpaceMembersProjection>,
    outcome: &'static str,
) {
    let output_count = projection.space_joined.len()
        + projection.space_invited.len()
        + projection.child_room_only.len();
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.space_members_projection",
        "projection",
    )
    .field(DiagnosticField::token("trigger", trigger))
    .field(DiagnosticField::count("generation", generation))
    .field(DiagnosticField::count(
        "space_joined_count",
        projection.space_joined.len() as u64,
    ))
    .field(DiagnosticField::count(
        "space_invited_count",
        projection.space_invited.len() as u64,
    ))
    .field(DiagnosticField::count(
        "child_room_only_count",
        projection.child_room_only.len() as u64,
    ))
    .field(DiagnosticField::count(
        "child_room_count",
        projection.child_room_count as u64,
    ))
    .field(DiagnosticField::count(
        "complete_child_room_count",
        projection.complete_child_room_count as u64,
    ))
    .field(DiagnosticField::count(
        "incomplete_child_room_count",
        projection.incomplete_child_room_count as u64,
    ))
    .field(DiagnosticField::count("output_count", output_count as u64))
    .field(DiagnosticField::count(
        "space_joined_output_count",
        projection.space_joined.len() as u64,
    ))
    .field(DiagnosticField::count(
        "space_invited_output_count",
        projection.space_invited.len() as u64,
    ))
    .field(DiagnosticField::count(
        "child_room_only_output_count",
        projection.child_room_only.len() as u64,
    ))
    .field(DiagnosticField::boolean(
        "incomplete",
        projection.incomplete_child_room_count > 0,
    ))
    .field(DiagnosticField::token("outcome", outcome));

    if let Some(raw_projection) = raw_projection {
        let input_count = raw_projection.space_joined_input_count
            + raw_projection.space_invited_input_count
            + raw_projection.child_join_input_count;
        event = event
            .field(DiagnosticField::count("input_count", input_count as u64))
            .field(DiagnosticField::count(
                "space_joined_input_count",
                raw_projection.space_joined_input_count as u64,
            ))
            .field(DiagnosticField::count(
                "space_invited_input_count",
                raw_projection.space_invited_input_count as u64,
            ))
            .field(DiagnosticField::count(
                "child_join_input_count",
                raw_projection.child_join_input_count as u64,
            ))
            .field(DiagnosticField::count(
                "deduplicated_count",
                raw_projection.duplicate_child_membership_count as u64,
            ))
            .field(DiagnosticField::count(
                "child_join_union_count",
                raw_projection.child_join_union_count as u64,
            ))
            .field(DiagnosticField::token("input_tracking_status", "tracked"));
    } else {
        event = event
            .field(DiagnosticField::token("input_count", "not_tracked"))
            .field(DiagnosticField::token("deduplicated_count", "not_tracked"))
            .field(DiagnosticField::token(
                "input_tracking_status",
                "not_tracked",
            ));
    }

    record(event.field(DiagnosticField::token("freshness_status", "not_tracked")));
}

fn space_members_update_affects_demand(
    space_id: &str,
    child_room_ids: &BTreeSet<String>,
    updated_room_ids: Option<&BTreeSet<String>>,
) -> bool {
    updated_room_ids.map_or(true, |updated| {
        updated.contains(space_id)
            || updated
                .iter()
                .any(|room_id| child_room_ids.contains(room_id))
    })
}

fn should_clear_space_member_demand(
    demand: Option<&SpaceMemberDemand>,
    space_id: &str,
    generation: u64,
) -> bool {
    demand.is_some_and(|demand| demand.space_id != space_id || demand.generation != generation)
}

fn space_members_refresh_is_current(
    result_space_id: &str,
    result_generation: u64,
    demanded_space_id: &str,
    demanded_generation: u64,
) -> bool {
    result_space_id == demanded_space_id && result_generation == demanded_generation
}

fn space_member_refresh_fence_is_current(
    active_fence: Option<SpaceMemberRefreshFence>,
    expected_fence: SpaceMemberRefreshFence,
    current_session_generation: u64,
    current_demand_generation: u64,
    result_space_id: &str,
    result_generation: u64,
    demanded_space_id: &str,
    demanded_generation: u64,
) -> bool {
    active_fence == Some(expected_fence)
        && current_session_generation == expected_fence.session_generation
        && current_demand_generation == expected_fence.demand_generation
        && space_members_refresh_is_current(
            result_space_id,
            result_generation,
            demanded_space_id,
            demanded_generation,
        )
}

fn record_space_member_demand_event(
    outcome: &'static str,
    generation: u64,
    child_room_count: usize,
) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.space_members_projection",
            "demand",
        )
        .field(DiagnosticField::token("outcome", outcome))
        .field(DiagnosticField::count("generation", generation))
        .field(DiagnosticField::count(
            "child_room_count",
            child_room_count as u64,
        )),
    );
}

fn record_space_member_refresh_event(outcome: &'static str, applied: bool) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.space_members_projection",
            "background_refresh",
        )
        .field(DiagnosticField::token("outcome", outcome))
        .field(DiagnosticField::boolean("applied", applied)),
    );
}

fn record_core_space_members_load_failure(trigger: &'static str, generation: u64) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.space_members_projection",
            "projection",
        )
        .field(DiagnosticField::token("trigger", trigger))
        .field(DiagnosticField::count("generation", generation))
        .field(DiagnosticField::token("outcome", "lookup_failed"))
        .field(DiagnosticField::token(
            "space_joined_count_availability",
            "counts_unavailable",
        ))
        .field(DiagnosticField::token(
            "space_invited_count_availability",
            "counts_unavailable",
        ))
        .field(DiagnosticField::token(
            "child_count_availability",
            "counts_unavailable",
        ))
        .field(DiagnosticField::token("input_count", "counts_unavailable"))
        .field(DiagnosticField::token("output_count", "counts_unavailable"))
        .field(DiagnosticField::token("freshness_status", "not_tracked")),
    );
}

fn space_member_role_failure_kind(
    kind: koushi_sdk::MatrixSpaceMemberRoleFailureKind,
) -> koushi_state::SpaceMemberRoleFailureKind {
    match kind {
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::Forbidden => {
            koushi_state::SpaceMemberRoleFailureKind::Forbidden
        }
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::Stale => {
            koushi_state::SpaceMemberRoleFailureKind::Stale
        }
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::NotFound => {
            koushi_state::SpaceMemberRoleFailureKind::NotFound
        }
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::Network => {
            koushi_state::SpaceMemberRoleFailureKind::Network
        }
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::Timeout => {
            koushi_state::SpaceMemberRoleFailureKind::Timeout
        }
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::Invalid => {
            koushi_state::SpaceMemberRoleFailureKind::Invalid
        }
        koushi_sdk::MatrixSpaceMemberRoleFailureKind::Sdk => {
            koushi_state::SpaceMemberRoleFailureKind::Sdk
        }
    }
}

fn space_member_role_failure_from_error(
    error: &MatrixRoomOperationError,
) -> koushi_state::SpaceMemberRoleFailureKind {
    match error {
        MatrixRoomOperationError::InvalidUserId
        | MatrixRoomOperationError::InvalidRoomId
        | MatrixRoomOperationError::InvalidRoomSetting => {
            koushi_state::SpaceMemberRoleFailureKind::Invalid
        }
        MatrixRoomOperationError::RoomUnavailable => {
            koushi_state::SpaceMemberRoleFailureKind::NotFound
        }
        MatrixRoomOperationError::Sdk(kind) => match kind {
            koushi_sdk::MatrixRoomOperationFailureKind::Forbidden
            | koushi_sdk::MatrixRoomOperationFailureKind::AuthenticationRequired => {
                koushi_state::SpaceMemberRoleFailureKind::Forbidden
            }
            koushi_sdk::MatrixRoomOperationFailureKind::Http => {
                koushi_state::SpaceMemberRoleFailureKind::Network
            }
            _ => koushi_state::SpaceMemberRoleFailureKind::Sdk,
        },
        _ => koushi_state::SpaceMemberRoleFailureKind::Sdk,
    }
}

fn record_core_space_members_operation(
    trigger: &'static str,
    generation: u64,
    outcome: &SpaceMemberInviteOutcome,
) {
    let outcome_token = match outcome {
        SpaceMemberInviteOutcome::Invited => "invited",
        SpaceMemberInviteOutcome::AlreadyInvited => "already_invited",
        SpaceMemberInviteOutcome::AlreadyJoined => "already_joined",
        SpaceMemberInviteOutcome::Cancelled => "cancelled",
        SpaceMemberInviteOutcome::NotInvited => "not_invited",
        SpaceMemberInviteOutcome::Failed(_) => "failed",
    };
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.space_members_projection",
            "invite_settled",
        )
        .field(DiagnosticField::token("trigger", trigger))
        .field(DiagnosticField::count("generation", generation))
        .field(DiagnosticField::token("outcome", outcome_token)),
    );
}

#[cfg(test)]
fn record_core_profile_resolution(projection: &SpaceMembersProjection) {
    let entries = projection
        .space_joined
        .iter()
        .chain(projection.space_invited.iter())
        .chain(projection.child_room_only.iter());
    let mut counts = [0_u64; 7];
    let input_count = entries
        .map(|entry| {
            let (relevant_room_label, space_room_label) =
                match entry.membership {
                    SpaceMemberMembership::ChildRoomOnly => (
                        entry.display_name.as_deref().filter(|label| {
                            !label.trim().is_empty() && label.trim() != "Unknown user"
                        }),
                        None,
                    ),
                    SpaceMemberMembership::SpaceJoined | SpaceMemberMembership::SpaceInvited => (
                        None,
                        entry.display_name.as_deref().filter(|label| {
                            !label.trim().is_empty() && label.trim() != "Unknown user"
                        }),
                    ),
                };
            let resolution = resolve_people_label(ProfileResolutionInput {
                local_alias: None,
                relevant_room_label,
                space_room_label,
                payload_label: None,
                cached_label: None,
                local_homeserver_label: None,
            });
            let index = match resolution.source {
                ProfileResolutionSource::LocalAlias => 0,
                ProfileResolutionSource::RelevantRoom => 1,
                ProfileResolutionSource::SpaceRoom => 2,
                ProfileResolutionSource::Payload => 3,
                ProfileResolutionSource::GlobalCache => 4,
                ProfileResolutionSource::LocalHomeserver => 5,
                ProfileResolutionSource::Unresolved => 6,
            };
            counts[index] += 1;
        })
        .count() as u64;
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.profile_resolution",
            "space_member_projection",
        )
        .field(DiagnosticField::count("input_count", input_count))
        .field(DiagnosticField::count("output_count", input_count))
        .field(DiagnosticField::count("local_alias_count", counts[0]))
        .field(DiagnosticField::count("relevant_room_count", counts[1]))
        .field(DiagnosticField::count("space_room_count", counts[2]))
        .field(DiagnosticField::count("payload_count", counts[3]))
        .field(DiagnosticField::count("global_cache_count", counts[4]))
        .field(DiagnosticField::count("local_homeserver_count", counts[5]))
        .field(DiagnosticField::count("unresolved_count", counts[6]))
        .field(DiagnosticField::token(
            "cache_stale_hit_status",
            "not_tracked",
        ))
        .field(DiagnosticField::token(
            "cache_freshness_status",
            "not_tracked",
        )),
    );
}

impl RoomActor {
    pub(super) async fn handle_load_space_members(
        &mut self,
        request_id: RequestId,
        space_id: String,
        generation: u64,
    ) {
        // Keep a same-Space/generation demand installed while this explicit
        // load is in flight so a failed retry cannot lose sync refreshes. A
        // different demand still supersedes the previous Space immediately.
        if should_clear_space_member_demand(
            self.space_member_demand.as_ref(),
            &space_id,
            generation,
        ) {
            self.clear_space_member_demand();
        }
        let Some(session) = self.session.clone() else {
            let kind = OperationFailureKind::Sdk;
            self.reduce_reliable(vec![AppAction::SpaceMembersLoadFailed {
                request_id: request_id.sequence,
                space_id,
                generation,
                kind,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::matrix_space_members_projection(&session, &space_id).await {
            Ok(raw_projection) => {
                self.install_space_member_demand(
                    &space_id,
                    generation,
                    &raw_projection.child_room_ids,
                );
                let profile_updates = user_profiles_from_space_projection(&raw_projection);
                let projection = state_space_members_projection(raw_projection.clone(), generation);
                record_core_space_members_projection_with_raw(
                    "load",
                    generation,
                    &raw_projection,
                    &projection,
                    "success",
                );
                self.reduce_reliable(vec![AppAction::SpaceMembersProjectionReconciled {
                    request_id: request_id.sequence,
                    projection: projection.clone(),
                    profiles: profile_updates,
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::SpaceMembersLoaded {
                    request_id,
                    generation,
                    joined_count: projection.space_joined.len(),
                    invited_count: projection.space_invited.len(),
                    child_room_only_count: projection.child_room_only.len(),
                    incomplete_child_room_count: projection.incomplete_child_room_count,
                }));
            }
            Err(error) => {
                let kind = operation_failure_kind(classify_room_error(&error));
                record_core_space_members_load_failure("load", generation);
                self.reduce_reliable(vec![AppAction::SpaceMembersLoadFailed {
                    request_id: request_id.sequence,
                    space_id,
                    generation,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::RoomOperationFailed {
                        kind: classify_room_error(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_space_membership_changed(
        &mut self,
        room_ids: Option<&BTreeSet<String>>,
    ) {
        let Some(demand) = self.space_member_demand.clone() else {
            return;
        };
        if !space_members_update_affects_demand(&demand.space_id, &demand.child_room_ids, room_ids)
        {
            return;
        }
        if self.space_member_refresh_in_flight.is_some() {
            self.space_member_refresh_pending = true;
            return;
        }
        self.start_space_member_refresh(demand);
    }

    fn start_space_member_refresh(&mut self, demand: SpaceMemberDemand) {
        let Some(session) = self.session.clone() else {
            return;
        };

        self.space_member_refresh_sequence =
            self.space_member_refresh_sequence.wrapping_add(1).max(1);
        let refresh_generation = self.space_member_refresh_sequence;
        let request_id = RequestId {
            connection_id: SPACE_MEMBER_REFRESH_CONNECTION_ID,
            sequence: refresh_generation,
        };
        let session_generation = self.space_member_session_generation;
        let fence = SpaceMemberRefreshFence {
            request_id,
            session_generation,
            demand_generation: demand.demand_generation,
            refresh_generation,
        };
        self.space_member_refresh_in_flight = Some(fence);

        let room_tx = self.self_tx.clone();
        let space_id = demand.space_id.clone();
        let generation = demand.generation;
        let demand_generation = demand.demand_generation;
        let _ = executor::spawn(async move {
            let result = koushi_sdk::matrix_space_members_projection(&session, &space_id).await;
            let _ = room_tx
                .send(RoomMessage::SpaceMembersProjectionRefreshed {
                    request_id,
                    session_generation,
                    demand_generation,
                    refresh_generation,
                    space_id,
                    generation,
                    result,
                })
                .await;
        });
    }

    pub(super) async fn handle_space_members_projection_refreshed(
        &mut self,
        request_id: RequestId,
        session_generation: u64,
        demand_generation: u64,
        refresh_generation: u64,
        space_id: String,
        generation: u64,
        result: Result<MatrixSpaceMembersProjection, MatrixRoomOperationError>,
    ) {
        let Some(demand) = self.space_member_demand.clone() else {
            return;
        };
        let is_current = space_member_refresh_fence_is_current(
            self.space_member_refresh_in_flight,
            SpaceMemberRefreshFence {
                request_id,
                session_generation,
                demand_generation,
                refresh_generation,
            },
            self.space_member_session_generation,
            demand.demand_generation,
            &space_id,
            generation,
            &demand.space_id,
            demand.generation,
        );
        if !is_current {
            record_space_member_refresh_event("stale_completion_ignored", false);
            return;
        }

        self.space_member_refresh_in_flight = None;
        let should_refresh_again = std::mem::take(&mut self.space_member_refresh_pending);
        match result {
            Ok(raw_projection) => {
                let profiles = user_profiles_from_space_projection(&raw_projection);
                let projection = state_space_members_projection(raw_projection.clone(), generation);
                record_core_space_members_projection_with_raw(
                    "sync_refresh",
                    generation,
                    &raw_projection,
                    &projection,
                    "success",
                );
                self.reduce_reliable(vec![
                    AppAction::SpaceMembersBackgroundProjectionReconciled {
                        request_id: request_id.sequence,
                        space_id: space_id.clone(),
                        generation,
                        projection,
                        profiles,
                    },
                ])
                .await;
                self.install_space_member_demand(
                    &space_id,
                    generation,
                    &raw_projection.child_room_ids,
                );
            }
            Err(_error) => {
                // A background lookup failure is deliberately silent at the
                // state layer: the last-known projection remains visible and
                // the next relevant sync update may retry it.
                record_core_space_members_load_failure("sync_refresh", generation);
            }
        }

        if should_refresh_again {
            if let Some(demand) = self.space_member_demand.clone() {
                self.start_space_member_refresh(demand);
            }
        }
    }

    pub(super) async fn handle_invite_user_to_space(
        &self,
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
    ) {
        let (outcome, reconciliation) = match &self.session {
            None => (
                SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
                None,
            ),
            Some(session) => {
                match koushi_sdk::invite_user_to_room(session, &space_id, &user_id).await {
                    Ok(()) => {
                        let fallback = SpaceMemberInviteOutcome::Invited;
                        match reconcile_space_invite_outcome(
                            session,
                            &space_id,
                            &user_id,
                            generation,
                            fallback.clone(),
                        )
                        .await
                        {
                            Some(reconciliation) => {
                                (reconciliation.outcome.clone(), Some(reconciliation))
                            }
                            None => (fallback, None),
                        }
                    }
                    Err(error) => {
                        let failure_kind = operation_failure_kind(classify_room_error(&error));
                        let fallback = SpaceMemberInviteOutcome::Failed(failure_kind);
                        match reconcile_space_invite_outcome(
                            session,
                            &space_id,
                            &user_id,
                            generation,
                            fallback.clone(),
                        )
                        .await
                        {
                            Some(reconciliation) => {
                                (reconciliation.outcome.clone(), Some(reconciliation))
                            }
                            None => (fallback, None),
                        }
                    }
                }
            }
        };
        record_core_space_members_operation("invite", generation, &outcome);
        let mut actions = Vec::new();
        if let Some(reconciliation) = reconciliation {
            actions.push(AppAction::SpaceMembersProjectionReconciled {
                request_id: request_id.sequence,
                projection: reconciliation.projection,
                profiles: reconciliation.profiles,
            });
        }
        actions.push(AppAction::SpaceMemberInviteSettled {
            request_id: request_id.sequence,
            space_id: space_id.clone(),
            user_id: user_id.clone(),
            generation,
            outcome: outcome.clone(),
        });
        self.reduce_reliable(actions).await;
        self.emit(CoreEvent::Room(RoomEvent::SpaceMemberInviteSettled {
            request_id,
            space_id,
            user_id,
            generation,
            outcome,
        }));
    }

    pub(super) async fn handle_update_space_member_role(
        &self,
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
        expected_power_levels_revision: Option<String>,
        expected_power_level: i64,
        power_level: i64,
        confirmed: bool,
    ) {
        let result = match &self.session {
            Some(session) => {
                koushi_sdk::update_space_member_power_level(
                    session,
                    &space_id,
                    &user_id,
                    expected_power_levels_revision.as_deref(),
                    expected_power_level,
                    power_level,
                    confirmed,
                )
                .await
            }
            None => Err(MatrixRoomOperationError::RoomUnavailable),
        };
        let (outcome, sent_revision, projection) = match result {
            Ok(result) => {
                let outcome = if result.succeeded {
                    koushi_state::SpaceMemberRoleUpdateOutcome::Succeeded
                } else {
                    koushi_state::SpaceMemberRoleUpdateOutcome::Failed(
                        result
                            .failure_kind
                            .map(space_member_role_failure_kind)
                            .unwrap_or(koushi_state::SpaceMemberRoleFailureKind::Sdk),
                    )
                };
                let projection = result
                    .projection
                    .map(|projection| state_space_members_projection(projection, generation));
                (outcome, result.sent_revision, projection)
            }
            Err(error) => (
                koushi_state::SpaceMemberRoleUpdateOutcome::Failed(
                    space_member_role_failure_from_error(&error),
                ),
                None,
                None,
            ),
        };
        self.reduce_reliable(vec![AppAction::SpaceMemberRoleUpdateSettled {
            request_id: request_id.sequence,
            space_id: space_id.clone(),
            user_id: user_id.clone(),
            generation,
            outcome: outcome.clone(),
            sent_revision,
            projection,
        }])
        .await;
        self.emit(CoreEvent::Room(RoomEvent::SpaceMemberRoleUpdateSettled {
            request_id,
            space_id,
            user_id,
            generation,
            outcome,
        }));
    }

    pub(super) async fn handle_cancel_space_invite(
        &self,
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
    ) {
        let (outcome, reconciliation) = match &self.session {
            None => (
                SpaceMemberInviteOutcome::Failed(OperationFailureKind::Sdk),
                None,
            ),
            Some(session) => {
                let outcome =
                    match koushi_sdk::cancel_space_invite(session, &space_id, &user_id).await {
                        Ok(koushi_sdk::MatrixSpaceInviteCancellationOutcome::Cancelled) => {
                            SpaceMemberInviteOutcome::Cancelled
                        }
                        Ok(koushi_sdk::MatrixSpaceInviteCancellationOutcome::NotInvited) => {
                            SpaceMemberInviteOutcome::NotInvited
                        }
                        Err(error) => SpaceMemberInviteOutcome::Failed(operation_failure_kind(
                            classify_room_error(&error),
                        )),
                    };
                let reconciliation =
                    reconcile_space_invite_cancellation(session, &space_id, generation).await;
                (outcome, reconciliation)
            }
        };
        record_core_space_members_operation("cancel", generation, &outcome);
        let mut actions = Vec::new();
        if let Some(reconciliation) = reconciliation {
            actions.push(AppAction::SpaceMembersProjectionReconciled {
                request_id: request_id.sequence,
                projection: reconciliation.projection,
                profiles: reconciliation.profiles,
            });
        }
        actions.push(AppAction::SpaceMemberInviteCancellationSettled {
            request_id: request_id.sequence,
            space_id: space_id.clone(),
            user_id: user_id.clone(),
            generation,
            outcome: outcome.clone(),
        });
        self.reduce_reliable(actions).await;
        self.emit(CoreEvent::Room(
            RoomEvent::SpaceMemberInviteCancellationSettled {
                request_id,
                space_id,
                user_id,
                generation,
                outcome,
            },
        ));
    }

    pub(super) fn reset_space_member_session(&mut self) {
        self.space_member_session_generation =
            self.space_member_session_generation.wrapping_add(1).max(1);
        self.clear_space_member_demand();
    }

    fn clear_space_member_demand(&mut self) {
        self.space_member_demand = None;
        self.space_member_demand_generation =
            self.space_member_demand_generation.wrapping_add(1).max(1);
        self.space_member_refresh_in_flight = None;
        self.space_member_refresh_pending = false;
        self.space_member_refresh_sequence = 0;
    }

    fn install_space_member_demand(
        &mut self,
        space_id: &str,
        generation: u64,
        child_room_ids: &[String],
    ) {
        self.space_member_demand_generation =
            self.space_member_demand_generation.wrapping_add(1).max(1);
        let child_room_ids = child_room_ids.iter().cloned().collect::<BTreeSet<_>>();
        let child_room_count = child_room_ids.len();
        self.space_member_demand = Some(SpaceMemberDemand {
            space_id: space_id.to_owned(),
            generation,
            child_room_ids,
            demand_generation: self.space_member_demand_generation,
        });
        record_space_member_demand_event("installed", generation, child_room_count);
    }
}

#[cfg(test)]
mod tests;
