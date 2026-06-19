use std::{collections::{BTreeMap, BTreeSet}, fmt};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AvatarImage {
    pub mxc_uri: String,
    pub thumbnail: AvatarThumbnailState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AvatarThumbnailState {
    #[default]
    NotRequested,
    Loading {
        request_id: u64,
    },
    Ready {
        source_url: String,
        width: Option<u64>,
        height: Option<u64>,
        mime_type: Option<String>,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: AvatarThumbnailFailureKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AvatarThumbnailFailureKind {
    Network,
    Forbidden,
    Unsupported,
    Sdk,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileState {
    pub own: OwnProfile,
    pub users: BTreeMap<String, UserProfile>,
    #[serde(default)]
    pub local_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub local_alias_update: LocalUserAliasUpdateState,
    #[serde(default)]
    pub ignored_user_ids: BTreeSet<String>,
    #[serde(default)]
    pub ignored_user_update: IgnoredUserUpdateState,
    pub update: ProfileUpdateState,
}

impl fmt::Debug for ProfileState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileState")
            .field("has_own_display_name", &self.own.display_name.is_some())
            .field("has_own_avatar", &self.own.avatar.is_some())
            .field("user_count", &self.users.len())
            .field("local_alias_count", &self.local_aliases.len())
            .field("local_alias_update", &self.local_alias_update)
            .field("ignored_user_count", &self.ignored_user_ids.len())
            .field("ignored_user_update", &self.ignored_user_update)
            .field("update", &self.update)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnProfile {
    pub display_name: Option<String>,
    pub avatar: Option<AvatarImage>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    #[serde(default)]
    pub mention_search_terms: Vec<String>,
    pub avatar: Option<AvatarImage>,
}

impl fmt::Debug for UserProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserProfile")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field("mention_search_terms", &self.mention_search_terms.len())
            .field("has_avatar", &self.avatar.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalUserAliasUpdateState {
    #[default]
    Idle,
    Saving {
        request_id: u64,
    },
}

impl LocalUserAliasUpdateState {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Saving { request_id } => Some(*request_id),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IgnoredUserUpdateState {
    #[default]
    Idle,
    Saving {
        request_id: u64,
    },
}

impl IgnoredUserUpdateState {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Saving { request_id } => Some(*request_id),
        }
    }
}

pub fn normalize_local_user_alias(alias: Option<String>) -> Option<String> {
    alias.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

pub fn is_ignored_user(profile: &ProfileState, user_id: Option<&str>) -> bool {
    user_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .is_some_and(|id| profile.ignored_user_ids.contains(id))
}

pub fn resolve_user_display_name(
    profiles: &ProfileState,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    let upstream_display_name = upstream_display_name
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty());
    let display_name = upstream_display_name.or_else(|| {
        profiles
            .users
            .get(user_id)
            .and_then(|profile| profile.display_name.as_deref())
    });
    resolve_user_display_name_from_parts(
        &profiles.local_aliases,
        profiles.own.display_name.as_deref(),
        user_id,
        display_name,
        own_user_id,
    )
}

pub fn original_user_display_name(
    profiles: &ProfileState,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    let upstream_display_name = upstream_display_name
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty());
    let display_name = upstream_display_name.or_else(|| {
        profiles
            .users
            .get(user_id)
            .and_then(|profile| profile.display_name.as_deref())
    });
    original_user_display_name_from_parts(
        profiles.own.display_name.as_deref(),
        user_id,
        display_name,
        own_user_id,
    )
}

pub fn refresh_profile_user_display_projection(
    profiles: &mut ProfileState,
    own_user_id: Option<&str>,
) {
    let local_aliases = &profiles.local_aliases;
    let own_display_name = profiles.own.display_name.as_deref();
    for (user_id, profile) in &mut profiles.users {
        let original_display_label = original_user_display_name_from_parts(
            own_display_name,
            user_id,
            profile.display_name.as_deref(),
            own_user_id,
        );
        let display_label = resolve_user_display_name_from_parts(
            local_aliases,
            own_display_name,
            user_id,
            profile.display_name.as_deref(),
            own_user_id,
        );
        profile.mention_search_terms = user_mention_search_terms(
            display_label.clone(),
            original_display_label.clone(),
            user_id,
        );
        profile.original_display_label = original_display_label;
        profile.display_label = display_label;
    }
}

pub fn refresh_room_settings_member_display_projection(
    settings: &mut super::room_management::RoomSettingsSnapshot,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> bool {
    let mut changed = false;
    for member in &mut settings.members {
        let display_label = resolve_user_display_name(
            profiles,
            &member.user_id,
            member.display_name.as_deref(),
            own_user_id,
        );
        let original_display_label = original_user_display_name(
            profiles,
            &member.user_id,
            member.display_name.as_deref(),
            own_user_id,
        );
        if member.display_label != display_label
            || member.original_display_label != original_display_label
        {
            member.display_label = display_label;
            member.original_display_label = original_display_label;
            changed = true;
        }
    }
    changed
}

pub fn refresh_room_summary_display_projection(
    rooms: &mut [super::room::RoomSummary],
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> bool {
    let mut changed = false;
    for room in rooms {
        let (display_label, original_display_label) =
            projected_room_summary_display_labels(room, profiles, own_user_id);
        if room.display_label != display_label
            || room.original_display_label != original_display_label
        {
            room.display_label = display_label;
            room.original_display_label = original_display_label;
            changed = true;
        }
    }
    changed
}

fn projected_room_summary_display_labels(
    room: &super::room::RoomSummary,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> (String, String) {
    if room.is_dm
        && room.dm_user_ids.len() == 1
        && let Some(user_id) = room.dm_user_ids.first()
    {
        return (
            resolve_user_display_name(profiles, user_id, Some(&room.display_name), own_user_id),
            original_user_display_name(profiles, user_id, Some(&room.display_name), own_user_id),
        );
    }

    let display_label = room
        .display_name
        .trim()
        .is_empty()
        .then(|| room.room_id.clone())
        .unwrap_or_else(|| room.display_name.trim().to_owned());
    (display_label.clone(), display_label)
}

pub(crate) fn resolve_user_display_name_from_parts(
    local_aliases: &BTreeMap<String, String>,
    own_display_name: Option<&str>,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    local_aliases
        .get(user_id)
        .filter(|alias| !alias.trim().is_empty())
        .cloned()
        .or_else(|| {
            upstream_display_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            own_user_id
                .filter(|own| *own == user_id)
                .and_then(|_| own_display_name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| user_id.to_owned())
}

pub(crate) fn original_user_display_name_from_parts(
    own_display_name: Option<&str>,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    upstream_display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            own_user_id
                .filter(|own| *own == user_id)
                .and_then(|_| own_display_name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| user_id.to_owned())
}

fn user_mention_search_terms(
    display_label: String,
    original_display_label: String,
    user_id: &str,
) -> Vec<String> {
    let mut terms = Vec::new();
    push_unique_search_term(&mut terms, display_label);
    push_unique_search_term(&mut terms, original_display_label);
    push_unique_search_term(&mut terms, user_id.to_owned());
    terms
}

fn push_unique_search_term(terms: &mut Vec<String>, term: String) {
    if !terms.iter().any(|existing| existing == &term) {
        terms.push(term);
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProfileUpdateState {
    #[default]
    Idle,
    SettingDisplayName {
        request_id: u64,
        display_name: Option<String>,
    },
    SettingAvatar {
        request_id: u64,
        mime_type: String,
        byte_count: u64,
    },
}

impl ProfileUpdateState {
    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::SettingDisplayName { request_id, .. }
            | Self::SettingAvatar { request_id, .. } => Some(*request_id),
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProfileUpdateRequest {
    SetDisplayName { display_name: Option<String> },
    SetAvatar { mime_type: String, byte_count: u64 },
}
