//! The pure room-access/history outcome resolver (#1177).
//!
//! One function turns the effective access tuple (join policy, history policy,
//! effective encryption, directory publication, target facts and viewer facts)
//! into catalog ids plus substitutions. It serves the confirmed Room Info
//! projection and the draft preview; it never stores an outcome of its own.
//!
//! Server history eligibility and key availability are deliberately separate
//! facts: a user who is eligible to read a shared history may still lack the
//! decryption keys for events sent before they joined.

use serde::{Deserialize, Serialize};

use crate::{RestrictedConditions, RoomAllowTarget, RoomHistoryVisibility, RoomJoinRule};

/// Where a room is listed in the queried directory (#1177).
///
/// `Loading` is the initial state; `Unavailable` means the server does not
/// answer the directory query at all, `Failed` means this read failed. Only
/// `Public`/`Private` are confirmed values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomDirectoryVisibility {
    #[default]
    Loading,
    Public,
    Private,
    Unavailable,
    Failed,
    /// The create dialog's proposed listing. Never a confirmed observation.
    ProposedPublic,
    /// The create dialog's proposed non-listing. Never a confirmed observation.
    ProposedPrivate,
}

/// One resolved outcome line: a catalog id plus ordered substitutions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomAccessOutcomeLine {
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub substitutions: Vec<String>,
}

impl RoomAccessOutcomeLine {
    fn new(message_id: &str) -> Self {
        Self {
            message_id: message_id.to_owned(),
            substitutions: Vec::new(),
        }
    }

    fn with(mut self, substitution: impl Into<String>) -> Self {
        self.substitutions.push(substitution.into());
        self
    }
}

/// The resolved outcome of one effective access tuple (#1177).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomAccessOutcome {
    pub join: RoomAccessOutcomeLine,
    /// Present only when the viewer may request access under a request route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_request: Option<RoomAccessOutcomeLine>,
    pub history: RoomAccessOutcomeLine,
    pub encryption: RoomAccessOutcomeLine,
    pub directory: RoomAccessOutcomeLine,
    /// Server history eligibility is not key availability: present when
    /// encryption is on and history reaches past the member's own join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_key_caveat: Option<RoomAccessOutcomeLine>,
    /// Changing the policy does not rewrite the past or revoke shared keys.
    pub non_retroactive: RoomAccessOutcomeLine,
}

/// Viewer facts that change the outcome wording (#1177).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RoomAccessViewerFacts {
    /// Whether this viewer can request access under a request route.
    pub can_request_access: bool,
}

/// Every effective input of the outcome resolver (#1177).
///
/// `join_rule` is `None` when the rule content is unavailable. The restricted
/// completeness, the resolved single-Space route name and the directory
/// publication are authoritative facts supplied by callers, never React
/// assertions.
#[derive(Clone, Copy, Debug)]
pub struct RoomAccessResolveInput<'a> {
    pub join_rule: Option<RoomJoinRule>,
    pub restricted: Option<RestrictedConditions>,
    pub allow_targets: &'a [RoomAllowTarget],
    pub space_members_route: Option<&'a str>,
    pub history: RoomHistoryVisibility,
    pub encrypted: bool,
    pub directory: RoomDirectoryVisibility,
    pub viewer: RoomAccessViewerFacts,
}

/// Resolve the effective access tuple into catalog lines (#1177).
///
/// This is pure: callers supply every effective input, including the verified
/// restricted completeness, the resolved single-Space route name and the
/// confirmed directory publication. An absent `join_rule` is "not verified",
/// never a default rule.
pub fn resolve_room_access_outcome(input: RoomAccessResolveInput<'_>) -> RoomAccessOutcome {
    let RoomAccessResolveInput {
        join_rule,
        restricted,
        allow_targets,
        space_members_route,
        history,
        encrypted,
        directory,
        viewer,
    } = input;
    let route = space_members_route
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let has_targets = !allow_targets.is_empty();
    let mut join_request = None;
    let join = match join_rule {
        None => RoomAccessOutcomeLine::new("room.accessOutcomeJoinNotVerified"),
        Some(RoomJoinRule::Public) => RoomAccessOutcomeLine::new("room.accessOutcomeJoinPublic"),
        Some(RoomJoinRule::Invite) => RoomAccessOutcomeLine::new("room.accessOutcomeJoinInvite"),
        Some(RoomJoinRule::Private) => RoomAccessOutcomeLine::new("room.accessOutcomeJoinPrivate"),
        Some(RoomJoinRule::Knock) => RoomAccessOutcomeLine::new("room.accessOutcomeJoinKnock"),
        Some(RoomJoinRule::Unknown) => RoomAccessOutcomeLine::new("room.accessOutcomeJoinUnknown"),
        Some(RoomJoinRule::Restricted) | Some(RoomJoinRule::KnockRestricted) => {
            let knock_restricted = matches!(join_rule, Some(RoomJoinRule::KnockRestricted));
            if knock_restricted && viewer.can_request_access {
                join_request = Some(RoomAccessOutcomeLine::new(
                    "room.accessOutcomeJoinCanRequest",
                ));
            }
            match restricted {
                Some(RestrictedConditions::MembershipOnly) if route.is_some() && has_targets => {
                    RoomAccessOutcomeLine::new("room.accessOutcomeJoinSpaceMembers")
                        .with(route.unwrap_or_default())
                }
                Some(RestrictedConditions::MembershipOnly) => {
                    RoomAccessOutcomeLine::new("room.accessOutcomeJoinMembershipRoute")
                }
                Some(RestrictedConditions::ConfirmedEmpty) => {
                    RoomAccessOutcomeLine::new("room.accessOutcomeJoinNoRoute")
                }
                Some(RestrictedConditions::MembershipPlusUnsupported) => {
                    RoomAccessOutcomeLine::new("room.accessOutcomeJoinConditionsUnverifiedContent")
                }
                Some(RestrictedConditions::UnsupportedOnly) => {
                    RoomAccessOutcomeLine::new("room.accessOutcomeJoinUnsupportedContent")
                }
                Some(RestrictedConditions::NotInspected) | None => {
                    RoomAccessOutcomeLine::new("room.accessOutcomeJoinNotVerified")
                }
            }
        }
    };

    let history_line = match history {
        RoomHistoryVisibility::WorldReadable => {
            RoomAccessOutcomeLine::new("room.accessOutcomeHistoryWorldReadable")
        }
        RoomHistoryVisibility::Shared => {
            RoomAccessOutcomeLine::new("room.accessOutcomeHistoryShared")
        }
        RoomHistoryVisibility::Invited => {
            RoomAccessOutcomeLine::new("room.accessOutcomeHistoryInvited")
        }
        RoomHistoryVisibility::Joined => {
            RoomAccessOutcomeLine::new("room.accessOutcomeHistoryJoined")
        }
    };
    // Key availability is a different fact from server eligibility: an eligible
    // reader may hold no key for events sent before they joined.
    let history_key_caveat = (encrypted && history != RoomHistoryVisibility::Joined)
        .then(|| RoomAccessOutcomeLine::new("room.historySharedEncryptedHint"));

    let encryption = if encrypted {
        RoomAccessOutcomeLine::new("room.accessOutcomeEncrypted")
    } else {
        RoomAccessOutcomeLine::new("room.accessOutcomeNotEncrypted")
    };
    let directory = match directory {
        RoomDirectoryVisibility::Public => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryPublic")
        }
        RoomDirectoryVisibility::Private => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryPrivate")
        }
        RoomDirectoryVisibility::Unavailable => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryUnavailable")
        }
        RoomDirectoryVisibility::Failed => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryFailed")
        }
        RoomDirectoryVisibility::Loading => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryLoading")
        }
        RoomDirectoryVisibility::ProposedPublic => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryWillBePublic")
        }
        RoomDirectoryVisibility::ProposedPrivate => {
            RoomAccessOutcomeLine::new("room.accessOutcomeDirectoryWillBePrivate")
        }
    };

    RoomAccessOutcome {
        join,
        join_request,
        history: history_line,
        encryption,
        directory,
        history_key_caveat,
        non_retroactive: RoomAccessOutcomeLine::new("room.historyNonRetroactive"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RoomAllowTargetKind;

    fn target() -> RoomAllowTarget {
        RoomAllowTarget {
            kind: RoomAllowTargetKind::Space,
            room_id: "!space:example.invalid".to_owned(),
        }
    }

    fn resolve(
        rule: Option<RoomJoinRule>,
        restricted: Option<RestrictedConditions>,
        route: Option<&str>,
        history: RoomHistoryVisibility,
        encrypted: bool,
        directory: RoomDirectoryVisibility,
    ) -> RoomAccessOutcome {
        resolve_room_access_outcome(RoomAccessResolveInput {
            join_rule: rule,
            restricted,
            allow_targets: &[target()],
            space_members_route: route,
            history,
            encrypted,
            directory,
            viewer: RoomAccessViewerFacts::default(),
        })
    }

    #[test]
    fn verified_single_space_route_names_the_space() {
        let outcome = resolve(
            Some(RoomJoinRule::Restricted),
            Some(RestrictedConditions::MembershipOnly),
            Some("Design Team"),
            RoomHistoryVisibility::Shared,
            true,
            RoomDirectoryVisibility::Private,
        );
        assert_eq!(
            outcome.join.message_id,
            "room.accessOutcomeJoinSpaceMembers"
        );
        assert_eq!(outcome.join.substitutions, vec!["Design Team".to_owned()]);
        assert_eq!(
            outcome.directory.message_id,
            "room.accessOutcomeDirectoryPrivate"
        );
    }

    #[test]
    fn unsupported_and_unverified_content_stay_distinct() {
        let unsupported_only = resolve(
            Some(RoomJoinRule::Restricted),
            Some(RestrictedConditions::UnsupportedOnly),
            None,
            RoomHistoryVisibility::Shared,
            false,
            RoomDirectoryVisibility::Private,
        );
        assert_eq!(
            unsupported_only.join.message_id,
            "room.accessOutcomeJoinUnsupportedContent"
        );

        let not_inspected = resolve(
            None,
            Some(RestrictedConditions::NotInspected),
            None,
            RoomHistoryVisibility::Shared,
            false,
            RoomDirectoryVisibility::Loading,
        );
        assert_eq!(
            not_inspected.join.message_id,
            "room.accessOutcomeJoinNotVerified"
        );
        assert_eq!(
            not_inspected.directory.message_id,
            "room.accessOutcomeDirectoryLoading"
        );
        assert_eq!(
            resolve(
                Some(RoomJoinRule::Restricted),
                Some(RestrictedConditions::MembershipPlusUnsupported),
                None,
                RoomHistoryVisibility::Shared,
                false,
                RoomDirectoryVisibility::Failed,
            )
            .join
            .message_id,
            "room.accessOutcomeJoinConditionsUnverifiedContent"
        );
    }

    #[test]
    fn history_eligibility_is_separate_from_key_availability() {
        let shared_encrypted = resolve(
            Some(RoomJoinRule::Invite),
            None,
            None,
            RoomHistoryVisibility::Shared,
            true,
            RoomDirectoryVisibility::Private,
        );
        assert_eq!(
            shared_encrypted.history.message_id,
            "room.accessOutcomeHistoryShared"
        );
        assert_eq!(
            shared_encrypted
                .history_key_caveat
                .as_ref()
                .map(|line| line.message_id.as_str()),
            Some("room.historySharedEncryptedHint")
        );
        let joined = resolve(
            Some(RoomJoinRule::Invite),
            None,
            None,
            RoomHistoryVisibility::Joined,
            true,
            RoomDirectoryVisibility::Private,
        );
        assert!(joined.history_key_caveat.is_none());
        assert_eq!(
            joined.non_retroactive.message_id,
            "room.historyNonRetroactive"
        );
    }

    #[test]
    fn request_route_line_tracks_the_viewer_fact() {
        let outcome = resolve_room_access_outcome(RoomAccessResolveInput {
            join_rule: Some(RoomJoinRule::KnockRestricted),
            restricted: Some(RestrictedConditions::MembershipOnly),
            allow_targets: &[target()],
            space_members_route: Some("Design Team"),
            history: RoomHistoryVisibility::Invited,
            encrypted: false,
            directory: RoomDirectoryVisibility::Private,
            viewer: RoomAccessViewerFacts {
                can_request_access: true,
            },
        });
        assert_eq!(
            outcome
                .join_request
                .as_ref()
                .map(|line| line.message_id.as_str()),
            Some("room.accessOutcomeJoinCanRequest")
        );
        assert_eq!(
            outcome.encryption.message_id,
            "room.accessOutcomeNotEncrypted"
        );
    }
}
