use super::actor::{RoomActor, RoomMessage};
use super::operations::classify_room_error;
use crate::executor;
use crate::mention_candidates::{MentionMemberInput, project_candidates};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::failure::{CoreFailure, RoomFailureKind};
use koushi_protocol::ids::RequestId;
use koushi_sdk::{MatrixClientSession, MatrixJoinedMemberSnapshot, MatrixRoomOperationError};
use koushi_state::{
    AppAction, MentionCandidatesCompleteness, MentionCandidatesFailureKind, MentionSurface,
    RoomMentionPermission,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct MentionDemand {
    request_id: RequestId,
    generation: u64,
    query: String,
}

pub(super) fn user_profile_mention_search_terms(
    user_id: &str,
    display_name: Option<&str>,
) -> Vec<String> {
    let mut terms = Vec::new();
    if let Some(display_name) = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        terms.push(display_name.to_owned());
    }
    if !terms.iter().any(|term| term == user_id) {
        terms.push(user_id.to_owned());
    }
    terms
}

fn mention_failure_kind(error: &MatrixRoomOperationError) -> MentionCandidatesFailureKind {
    match classify_room_error(error) {
        RoomFailureKind::Forbidden => MentionCandidatesFailureKind::Forbidden,
        RoomFailureKind::Network => MentionCandidatesFailureKind::Network,
        RoomFailureKind::AliasInUse | RoomFailureKind::NotFound | RoomFailureKind::Sdk => {
            MentionCandidatesFailureKind::Sdk
        }
    }
}

fn record_mention_candidate_event(
    stage: &'static str,
    surface: MentionSurface,
    completeness: MentionCandidatesCompleteness,
    candidate_count: usize,
    outcome: &'static str,
) {
    let surface = match surface {
        MentionSurface::Main => "main",
        MentionSurface::Thread => "thread",
    };
    let completeness = match completeness {
        MentionCandidatesCompleteness::Loading => "loading",
        MentionCandidatesCompleteness::Partial => "partial",
        MentionCandidatesCompleteness::Complete => "complete",
        MentionCandidatesCompleteness::Failed => "failed",
    };
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "mention.candidates", stage)
            .field(DiagnosticField::token("surface", surface))
            .field(DiagnosticField::token("completeness", completeness))
            .field(DiagnosticField::count(
                "candidate_count",
                candidate_count as u64,
            ))
            .field(DiagnosticField::token("outcome", outcome)),
    );
}

impl RoomActor {
    pub(super) async fn handle_query_mention_candidates(
        &mut self,
        request_id: RequestId,
        account_key: koushi_protocol::AccountKey,
        room_id: String,
        surface: MentionSurface,
        query: String,
    ) {
        let Some(session) = self.session.clone() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        if account_key.0 != session.info.user_id {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }

        let key = (room_id.clone(), surface);
        let generation = self
            .mention_demands
            .get(&key)
            .map_or(1, |demand| demand.generation.wrapping_add(1).max(1));
        self.mention_demands.insert(
            key,
            MentionDemand {
                request_id,
                generation,
                query: query.clone(),
            },
        );
        self.reduce_reliable(vec![AppAction::MentionCandidatesDemanded {
            request_id: request_id.sequence,
            generation,
            room_id: room_id.clone(),
            surface,
            query: query.clone(),
        }])
        .await;
        record_mention_candidate_event(
            "requested",
            surface,
            MentionCandidatesCompleteness::Loading,
            0,
            "accepted",
        );

        match session.joined_member_snapshot_no_sync(&room_id).await {
            Ok(snapshot) => {
                self.mention_member_snapshots
                    .insert(room_id.clone(), snapshot.clone());
                self.publish_mention_projection(&room_id, surface, &snapshot)
                    .await;
                if !snapshot.complete {
                    self.start_mention_member_refresh(session, room_id);
                }
            }
            Err(error) => {
                self.publish_mention_failure(&room_id, surface, mention_failure_kind(&error))
                    .await;
            }
        }
    }

    pub(super) async fn handle_mention_members_refreshed(
        &mut self,
        room_id: String,
        session_generation: u64,
        refresh_generation: u64,
        result: Result<MatrixJoinedMemberSnapshot, MatrixRoomOperationError>,
    ) {
        if session_generation != self.mention_session_generation
            || self.mention_refresh_generations.get(&room_id) != Some(&refresh_generation)
            || self.session.is_none()
        {
            record_mention_candidate_event(
                "member_refresh_settled",
                MentionSurface::Main,
                MentionCandidatesCompleteness::Failed,
                0,
                "stale",
            );
            return;
        }
        self.mention_refresh_generations.remove(&room_id);
        let demanded_surfaces = self
            .mention_demands
            .keys()
            .filter_map(|(demanded_room_id, surface)| {
                (demanded_room_id == &room_id).then_some(*surface)
            })
            .collect::<Vec<_>>();
        match result {
            Ok(snapshot) => {
                self.mention_member_snapshots
                    .insert(room_id.clone(), snapshot.clone());
                for surface in demanded_surfaces {
                    self.publish_mention_projection(&room_id, surface, &snapshot)
                        .await;
                }
            }
            Err(error) => {
                let kind = mention_failure_kind(&error);
                for surface in demanded_surfaces {
                    self.publish_mention_failure(&room_id, surface, kind).await;
                }
            }
        }
    }

    fn start_mention_member_refresh(&mut self, session: Arc<MatrixClientSession>, room_id: String) {
        if self.mention_refresh_generations.contains_key(&room_id) {
            return;
        }
        self.mention_refresh_sequence = self.mention_refresh_sequence.wrapping_add(1).max(1);
        let refresh_generation = self.mention_refresh_sequence;
        self.mention_refresh_generations
            .insert(room_id.clone(), refresh_generation);
        let session_generation = self.mention_session_generation;
        let self_tx = self.self_tx.clone();
        record_mention_candidate_event(
            "member_refresh_started",
            MentionSurface::Main,
            MentionCandidatesCompleteness::Loading,
            0,
            "started",
        );
        executor::spawn(async move {
            let result = session.refresh_joined_member_snapshot(&room_id).await;
            let _ = self_tx
                .send(RoomMessage::MentionMembersRefreshed {
                    room_id,
                    session_generation,
                    refresh_generation,
                    result,
                })
                .await;
        });
    }

    pub(super) async fn handle_mention_membership_changed(
        &mut self,
        room_ids: Option<BTreeSet<String>>,
    ) {
        self.handle_space_membership_changed(room_ids.as_ref())
            .await;

        let demanded_rooms = self
            .mention_demands
            .keys()
            .filter_map(|(room_id, _)| {
                room_ids
                    .as_ref()
                    .is_none_or(|updated| updated.contains(room_id))
                    .then_some(room_id.clone())
            })
            .collect::<BTreeSet<_>>();
        let Some(session) = self.session.clone() else {
            return;
        };
        for room_id in demanded_rooms {
            self.mention_member_snapshots.remove(&room_id);
            // An update supersedes an in-flight refresh for this room. Its
            // completion is fenced by the replacement refresh generation.
            self.mention_refresh_generations.remove(&room_id);
            match session.joined_member_snapshot_no_sync(&room_id).await {
                Ok(snapshot) => {
                    self.mention_member_snapshots
                        .insert(room_id.clone(), snapshot.clone());
                    let surfaces = self
                        .mention_demands
                        .keys()
                        .filter_map(|(demanded_room_id, surface)| {
                            (demanded_room_id == &room_id).then_some(*surface)
                        })
                        .collect::<Vec<_>>();
                    for surface in surfaces {
                        self.publish_mention_projection(&room_id, surface, &snapshot)
                            .await;
                    }
                    if !snapshot.complete {
                        self.start_mention_member_refresh(session.clone(), room_id);
                    }
                }
                Err(error) => {
                    let kind = mention_failure_kind(&error);
                    let surfaces = self
                        .mention_demands
                        .keys()
                        .filter_map(|(demanded_room_id, surface)| {
                            (demanded_room_id == &room_id).then_some(*surface)
                        })
                        .collect::<Vec<_>>();
                    for surface in surfaces {
                        self.publish_mention_failure(&room_id, surface, kind).await;
                    }
                }
            }
        }
    }

    pub(super) async fn handle_mention_local_aliases_updated(
        &mut self,
        aliases: BTreeMap<String, String>,
    ) {
        self.mention_local_aliases = aliases;
        let demanded_targets = self.mention_demands.keys().cloned().collect::<Vec<_>>();
        for (room_id, surface) in demanded_targets {
            if let Some(snapshot) = self.mention_member_snapshots.get(&room_id).cloned() {
                self.publish_mention_projection(&room_id, surface, &snapshot)
                    .await;
            }
        }
    }

    async fn publish_mention_projection(
        &self,
        room_id: &str,
        surface: MentionSurface,
        snapshot: &MatrixJoinedMemberSnapshot,
    ) {
        let Some(demand) = self
            .mention_demands
            .get(&(room_id.to_owned(), surface))
            .cloned()
        else {
            return;
        };
        let permission = match snapshot.room_mention_allowed {
            Some(true) => RoomMentionPermission::Allowed,
            Some(false) => RoomMentionPermission::Denied,
            None => RoomMentionPermission::Unknown,
        };
        let projection = project_candidates(
            &demand.query,
            snapshot
                .members
                .iter()
                .map(|member| MentionMemberInput {
                    user_id: member.user_id.clone(),
                    room_display_name: member.display_name.clone(),
                    profile_display_name: None,
                    local_alias: self.mention_local_aliases.get(&member.user_id).cloned(),
                    avatar_mxc_uri: member.avatar_url.clone(),
                })
                .collect(),
            permission,
        );
        let room_mention_allowed = if projection.room_mention_included {
            RoomMentionPermission::Allowed
        } else if permission == RoomMentionPermission::Unknown {
            RoomMentionPermission::Unknown
        } else {
            RoomMentionPermission::Denied
        };
        let candidate_count = projection.candidates.len();
        self.reduce_reliable(vec![AppAction::MentionCandidatesProjected {
            request_id: demand.request_id.sequence,
            generation: demand.generation,
            room_id: room_id.to_owned(),
            surface,
            query: demand.query,
            completeness: if snapshot.complete {
                MentionCandidatesCompleteness::Complete
            } else {
                MentionCandidatesCompleteness::Partial
            },
            candidates: projection.candidates,
            room_mention_allowed,
        }])
        .await;
        record_mention_candidate_event(
            "projected",
            surface,
            if snapshot.complete {
                MentionCandidatesCompleteness::Complete
            } else {
                MentionCandidatesCompleteness::Partial
            },
            candidate_count,
            "success",
        );
    }

    async fn publish_mention_failure(
        &self,
        room_id: &str,
        surface: MentionSurface,
        kind: MentionCandidatesFailureKind,
    ) {
        let Some(demand) = self
            .mention_demands
            .get(&(room_id.to_owned(), surface))
            .cloned()
        else {
            return;
        };
        self.reduce_reliable(vec![AppAction::MentionCandidatesFailed {
            request_id: demand.request_id.sequence,
            generation: demand.generation,
            room_id: room_id.to_owned(),
            surface,
            query: demand.query,
            kind,
        }])
        .await;
        record_mention_candidate_event(
            "projected",
            surface,
            MentionCandidatesCompleteness::Failed,
            0,
            match kind {
                MentionCandidatesFailureKind::Network => "network",
                MentionCandidatesFailureKind::Forbidden => "forbidden",
                MentionCandidatesFailureKind::Sdk => "sdk",
            },
        );
    }

    pub(super) fn clear_mention_candidates(&mut self) {
        self.mention_demands.clear();
        self.mention_member_snapshots.clear();
        self.mention_refresh_generations.clear();
        self.mention_local_aliases.clear();
        self.mention_refresh_sequence = 0;
        self.mention_session_generation = self.mention_session_generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {}
