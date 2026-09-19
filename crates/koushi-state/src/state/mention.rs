use std::fmt;

use serde::{Deserialize, Serialize};

use super::AvatarImage;

pub const MAX_MENTION_CANDIDATE_TARGETS: usize = 6;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MentionSurface {
    Main,
    Thread,
    Edit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MentionCandidatesCompleteness {
    Loading,
    Partial,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomMentionPermission {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MentionCandidateMembership {
    Joined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MentionCandidatesFailureKind {
    Network,
    Forbidden,
    Sdk,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionCandidate {
    pub user_id: String,
    pub display_label: Option<String>,
    pub original_display_label: Option<String>,
    pub avatar: Option<AvatarImage>,
    pub membership: MentionCandidateMembership,
}

impl fmt::Debug for MentionCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MentionCandidate")
            .field("user_id", &"UserId(..)")
            .field(
                "has_display_label",
                &self
                    .display_label
                    .as_ref()
                    .is_some_and(|label| !label.is_empty()),
            )
            .field(
                "has_original_display_label",
                &self
                    .original_display_label
                    .as_ref()
                    .is_some_and(|label| !label.is_empty()),
            )
            .field("has_avatar", &self.avatar.is_some())
            .field("membership", &self.membership)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionCandidatesTarget {
    pub room_id: String,
    pub generation: u64,
    pub request_id: u64,
    pub query: String,
    pub surface: MentionSurface,
    pub completeness: MentionCandidatesCompleteness,
    pub candidates: Vec<MentionCandidate>,
    pub room_mention_allowed: RoomMentionPermission,
    pub failure_kind: Option<MentionCandidatesFailureKind>,
}

impl fmt::Debug for MentionCandidatesTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MentionCandidatesTarget")
            .field("room_id", &"RoomId(..)")
            .field("generation", &self.generation)
            .field("request_id", &self.request_id)
            .field("surface", &self.surface)
            .field("completeness", &self.completeness)
            .field("candidate_count", &self.candidates.len())
            .field("room_mention_allowed", &self.room_mention_allowed)
            .field("failure_kind", &self.failure_kind)
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionCandidatesState {
    pub targets: Vec<MentionCandidatesTarget>,
}

impl MentionCandidatesState {
    pub fn target(
        &self,
        room_id: &str,
        surface: MentionSurface,
    ) -> Option<&MentionCandidatesTarget> {
        self.targets
            .iter()
            .find(|target| target.room_id == room_id && target.surface == surface)
    }

    pub(crate) fn target_mut(
        &mut self,
        room_id: &str,
        surface: MentionSurface,
    ) -> Option<&mut MentionCandidatesTarget> {
        self.targets
            .iter_mut()
            .find(|target| target.room_id == room_id && target.surface == surface)
    }

    pub(crate) fn replace_target(&mut self, target: MentionCandidatesTarget) {
        self.targets.retain(|existing| {
            existing.room_id != target.room_id || existing.surface != target.surface
        });
        self.targets.push(target);
        if self.targets.len() > MAX_MENTION_CANDIDATE_TARGETS {
            let overflow = self.targets.len() - MAX_MENTION_CANDIDATE_TARGETS;
            self.targets.drain(..overflow);
        }
    }
}

impl fmt::Debug for MentionCandidatesState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MentionCandidatesState")
            .field("target_count", &self.targets.len())
            .finish()
    }
}
