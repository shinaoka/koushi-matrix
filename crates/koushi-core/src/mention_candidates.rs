use koushi_state::{
    AvatarImage, AvatarThumbnailState, MentionCandidate, MentionCandidateMembership,
    RoomMentionPermission, cjk_display_sort_key, normalize_cjk_search_text,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MentionMemberInput {
    pub user_id: String,
    pub room_display_name: Option<String>,
    pub profile_display_name: Option<String>,
    pub local_alias: Option<String>,
    pub avatar_mxc_uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MentionCandidatesProjection {
    pub candidates: Vec<MentionCandidate>,
    pub room_mention_included: bool,
}

pub(crate) fn project_candidates(
    query: &str,
    members: Vec<MentionMemberInput>,
    room_mention_permission: RoomMentionPermission,
) -> MentionCandidatesProjection {
    let normalized_query = normalize_query(query);
    let mut ranked = members
        .into_iter()
        .filter_map(|member| {
            let original_display_label = friendly_value(member.room_display_name.as_deref())
                .or_else(|| friendly_value(member.profile_display_name.as_deref()))
                .map(str::to_owned);
            let display_label = friendly_value(member.local_alias.as_deref())
                .map(str::to_owned)
                .or_else(|| original_display_label.clone());
            let localpart = matrix_localpart(&member.user_id);
            let terms = [
                friendly_value(member.local_alias.as_deref()),
                friendly_value(member.room_display_name.as_deref()),
                friendly_value(member.profile_display_name.as_deref()),
                Some(member.user_id.as_str()),
                localpart,
            ];
            let rank = match_rank(&normalized_query, terms.into_iter().flatten())?;
            let sort_label = display_label.as_deref().unwrap_or(member.user_id.as_str());
            let sort_key = cjk_display_sort_key(sort_label);
            let candidate = MentionCandidate {
                user_id: member.user_id.clone(),
                display_label,
                original_display_label,
                avatar: member.avatar_mxc_uri.map(|mxc_uri| AvatarImage {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
                membership: MentionCandidateMembership::Joined,
            };
            Some((rank, sort_key, member.user_id, candidate))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(
        |(left_rank, left_sort, left_id, _), (right_rank, right_sort, right_id, _)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| left_sort.cmp(right_sort))
                .then_with(|| left_id.cmp(right_id))
        },
    );

    MentionCandidatesProjection {
        candidates: ranked
            .into_iter()
            .map(|(_, _, _, candidate)| candidate)
            .collect(),
        room_mention_included: room_mention_permission == RoomMentionPermission::Allowed
            && match_rank(&normalized_query, ["room"].into_iter()).is_some(),
    }
}

fn friendly_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalize_query(query: &str) -> String {
    normalize_cjk_search_text(query.trim().strip_prefix('@').unwrap_or(query.trim()))
}

fn matrix_localpart(user_id: &str) -> Option<&str> {
    user_id
        .strip_prefix('@')
        .and_then(|value| value.split_once(':').map(|(localpart, _)| localpart))
        .filter(|localpart| !localpart.is_empty())
}

fn match_rank<'a>(normalized_query: &str, terms: impl Iterator<Item = &'a str>) -> Option<u8> {
    if normalized_query.is_empty() {
        return Some(2);
    }

    terms
        .map(normalize_cjk_search_text)
        .filter(|term| !term.is_empty())
        .filter_map(|term| {
            if term == normalized_query {
                Some(0)
            } else if term.starts_with(normalized_query)
                || term
                    .split_whitespace()
                    .any(|token| token.starts_with(normalized_query))
            {
                Some(1)
            } else if term.contains(normalized_query) {
                Some(2)
            } else {
                None
            }
        })
        .min()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod membership_tests;
