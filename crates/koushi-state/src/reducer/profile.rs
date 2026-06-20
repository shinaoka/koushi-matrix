use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppError, AppState, AvatarImage, AvatarThumbnailState, RoomListFilter},
};

use std::collections::BTreeMap;

use super::{
    is_session_ready, profile_changed_effects, recompute_room_list_projection,
    refresh_native_attention_candidate_display_projection,
    refresh_open_room_settings_member_display_projection,
    refresh_open_room_summary_display_projection, session_user_id,
};

const LOCAL_USER_ALIAS_UPDATE_FAILED_MESSAGE: &str = "Local user alias could not be saved";
const IGNORED_USER_UPDATE_FAILED_MESSAGE: &str = "Ignored user list could not be updated";

pub(crate) fn handle_own_profile_updated(
    state: &mut AppState,
    profile: crate::state::OwnProfile,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    state.profile.own = profile;
    crate::state::refresh_profile_user_display_projection(
        &mut state.profile,
        own_user_id.as_deref(),
    );
    let room_members_changed =
        refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
    let room_list_changed =
        refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
    let native_attention_changed =
        room_list_changed && refresh_native_attention_candidate_display_projection(state);
    profile_changed_effects(
        room_members_changed,
        room_list_changed,
        native_attention_changed,
    )
}

pub(crate) fn handle_user_profiles_updated(
    state: &mut AppState,
    profiles: Vec<crate::state::UserProfile>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    state.profile.users = profiles
        .into_iter()
        .map(|profile| (profile.user_id.clone(), profile))
        .collect();
    crate::state::refresh_profile_user_display_projection(
        &mut state.profile,
        own_user_id.as_deref(),
    );
    let room_members_changed =
        refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
    let room_list_changed =
        refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
    let native_attention_changed =
        room_list_changed && refresh_native_attention_candidate_display_projection(state);
    profile_changed_effects(
        room_members_changed,
        room_list_changed,
        native_attention_changed,
    )
}

pub(crate) fn handle_avatar_thumbnail_updated(
    state: &mut AppState,
    mxc_uri: String,
    thumbnail: AvatarThumbnailState,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let mut profile_changed = false;
    profile_changed |= update_avatar_thumbnail(&mut state.profile.own.avatar, &mxc_uri, &thumbnail);
    for profile in state.profile.users.values_mut() {
        profile_changed |= update_avatar_thumbnail(&mut profile.avatar, &mxc_uri, &thumbnail);
    }

    let mut room_list_changed = false;
    room_list_changed |= update_avatar_thumbnails_in_rooms(&mut state.rooms, &mxc_uri, &thumbnail);
    room_list_changed |= update_avatar_thumbnails_in_spaces(&mut state.spaces, &mxc_uri, &thumbnail);
    for invite in &mut state.invites {
        room_list_changed |= update_avatar_thumbnail(&mut invite.avatar, &mxc_uri, &thumbnail);
    }

    let mut effects = Vec::new();
    if profile_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ProfileChanged));
    }
    if room_list_changed {
        recompute_room_list_projection(state);
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    effects
}

pub(super) fn avatar_thumbnail_states_by_mxc(
    state: &AppState,
) -> BTreeMap<String, AvatarThumbnailState> {
    let mut states = BTreeMap::new();
    collect_avatar_thumbnail(&state.profile.own.avatar, &mut states);
    for profile in state.profile.users.values() {
        collect_avatar_thumbnail(&profile.avatar, &mut states);
    }
    for room in &state.rooms {
        collect_avatar_thumbnail(&room.avatar, &mut states);
    }
    for space in &state.spaces {
        collect_avatar_thumbnail(&space.avatar, &mut states);
    }
    for invite in &state.invites {
        collect_avatar_thumbnail(&invite.avatar, &mut states);
    }
    states
}

pub(super) fn apply_avatar_thumbnail_states_to_rooms_and_spaces(
    rooms: &mut [crate::state::RoomSummary],
    spaces: &mut [crate::state::SpaceSummary],
    states: &BTreeMap<String, AvatarThumbnailState>,
) {
    for room in rooms {
        apply_known_avatar_thumbnail(&mut room.avatar, states);
    }
    for space in spaces {
        apply_known_avatar_thumbnail(&mut space.avatar, states);
    }
}

fn collect_avatar_thumbnail(
    avatar: &Option<AvatarImage>,
    states: &mut BTreeMap<String, AvatarThumbnailState>,
) {
    let Some(avatar) = avatar else {
        return;
    };
    if avatar.thumbnail == AvatarThumbnailState::NotRequested {
        return;
    }
    states.insert(avatar.mxc_uri.clone(), avatar.thumbnail.clone());
}

fn apply_known_avatar_thumbnail(
    avatar: &mut Option<AvatarImage>,
    states: &BTreeMap<String, AvatarThumbnailState>,
) -> bool {
    let Some(avatar) = avatar else {
        return false;
    };
    let Some(thumbnail) = states.get(&avatar.mxc_uri) else {
        return false;
    };
    if avatar.thumbnail == *thumbnail {
        return false;
    }
    avatar.thumbnail = thumbnail.clone();
    true
}

fn update_avatar_thumbnails_in_rooms(
    rooms: &mut [crate::state::RoomSummary],
    mxc_uri: &str,
    thumbnail: &AvatarThumbnailState,
) -> bool {
    let mut changed = false;
    for room in rooms {
        changed |= update_avatar_thumbnail(&mut room.avatar, mxc_uri, thumbnail);
    }
    changed
}

fn update_avatar_thumbnails_in_spaces(
    spaces: &mut [crate::state::SpaceSummary],
    mxc_uri: &str,
    thumbnail: &AvatarThumbnailState,
) -> bool {
    let mut changed = false;
    for space in spaces {
        changed |= update_avatar_thumbnail(&mut space.avatar, mxc_uri, thumbnail);
    }
    changed
}

fn update_avatar_thumbnail(
    avatar: &mut Option<AvatarImage>,
    mxc_uri: &str,
    thumbnail: &AvatarThumbnailState,
) -> bool {
    let Some(avatar) = avatar else {
        return false;
    };
    if avatar.mxc_uri != mxc_uri || avatar.thumbnail == *thumbnail {
        return false;
    }
    avatar.thumbnail = thumbnail.clone();
    true
}

pub(crate) fn handle_local_user_aliases_loaded(
    state: &mut AppState,
    aliases: std::collections::BTreeMap<String, String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    state.profile.local_aliases = aliases
        .into_iter()
        .filter_map(|(user_id, alias)| {
            crate::state::normalize_local_user_alias(Some(alias))
                .map(|normalized| (user_id, normalized))
        })
        .collect();
    state.profile.local_alias_update = crate::state::LocalUserAliasUpdateState::Idle;
    crate::state::refresh_profile_user_display_projection(
        &mut state.profile,
        own_user_id.as_deref(),
    );
    let room_members_changed =
        refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
    let room_list_changed =
        refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
    let native_attention_changed =
        room_list_changed && refresh_native_attention_candidate_display_projection(state);
    profile_changed_effects(
        room_members_changed,
        room_list_changed,
        native_attention_changed,
    )
}

pub(crate) fn handle_local_user_alias_update_requested(
    state: &mut AppState,
    request_id: u64,
    user_id: String,
    alias: Option<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.profile.local_alias_update.is_idle() {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    if let Some(alias) = crate::state::normalize_local_user_alias(alias) {
        state.profile.local_aliases.insert(user_id, alias);
    } else {
        state.profile.local_aliases.remove(&user_id);
    }
    state.profile.local_alias_update =
        crate::state::LocalUserAliasUpdateState::Saving { request_id };
    crate::state::refresh_profile_user_display_projection(
        &mut state.profile,
        own_user_id.as_deref(),
    );
    let room_members_changed =
        refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
    let room_list_changed =
        refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
    let native_attention_changed =
        room_list_changed && refresh_native_attention_candidate_display_projection(state);
    profile_changed_effects(
        room_members_changed,
        room_list_changed,
        native_attention_changed,
    )
}

pub(crate) fn handle_local_user_alias_update_succeeded(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if state.profile.local_alias_update.request_id() != Some(request_id) {
        return Vec::new();
    }

    state.profile.local_alias_update = crate::state::LocalUserAliasUpdateState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)]
}

pub(crate) fn handle_local_user_alias_update_failed(
    state: &mut AppState,
    request_id: u64,
    _message: String,
) -> Vec<AppEffect> {
    if state.profile.local_alias_update.request_id() != Some(request_id) {
        return Vec::new();
    }

    state.profile.local_alias_update = crate::state::LocalUserAliasUpdateState::Idle;
    state.errors.push(AppError {
        code: "local_user_alias_update_failed".to_owned(),
        message: LOCAL_USER_ALIAS_UPDATE_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::ProfileChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_ignored_users_loaded(
    state: &mut AppState,
    user_ids: std::collections::BTreeSet<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.profile.ignored_user_ids = user_ids;
    state.profile.ignored_user_update = crate::state::IgnoredUserUpdateState::Idle;

    let own_user_id = session_user_id(state).map(str::to_owned);
    let ignored = &state.profile.ignored_user_ids;
    state
        .live_signals
        .presence
        .retain(|user_id, _| !ignored.contains(user_id));

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)];

    if state.room_list.active_filter == RoomListFilter::Invites {
        recompute_room_list_projection(state);
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }

    if !state.live_signals.presence.is_empty() || own_user_id.is_some() {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged));
    }

    effects
}

pub(crate) fn handle_ignored_user_update_requested(
    state: &mut AppState,
    request_id: u64,
    user_id: String,
    ignored: bool,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.profile.ignored_user_update.is_idle() {
        return Vec::new();
    }

    if ignored {
        state.profile.ignored_user_ids.insert(user_id.clone());
    } else {
        state.profile.ignored_user_ids.remove(&user_id);
    }
    state.profile.ignored_user_update = crate::state::IgnoredUserUpdateState::Saving { request_id };
    state.live_signals.presence.remove(&user_id);

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)];

    if state.room_list.active_filter == RoomListFilter::Invites {
        recompute_room_list_projection(state);
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }

    effects.push(AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged));
    effects
}

pub(crate) fn handle_ignored_user_update_succeeded(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if state.profile.ignored_user_update.request_id() != Some(request_id) {
        return Vec::new();
    }

    state.profile.ignored_user_update = crate::state::IgnoredUserUpdateState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)]
}

pub(crate) fn handle_ignored_user_update_failed(
    state: &mut AppState,
    request_id: u64,
    user_id: String,
    ignored: bool,
    _message: String,
) -> Vec<AppEffect> {
    if state.profile.ignored_user_update.request_id() != Some(request_id) {
        return Vec::new();
    }

    // Revert the optimistic mutation so the UI does not keep filtering
    // as if the failed operation succeeded.
    if ignored {
        state.profile.ignored_user_ids.remove(&user_id);
    } else {
        state.profile.ignored_user_ids.insert(user_id);
    }
    state.profile.ignored_user_update = crate::state::IgnoredUserUpdateState::Idle;
    state.errors.push(AppError {
        code: "ignored_user_update_failed".to_owned(),
        message: IGNORED_USER_UPDATE_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::ProfileChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_profile_update_requested(
    state: &mut AppState,
    request_id: u64,
    request: crate::state::ProfileUpdateRequest,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.profile.update.is_idle() {
        return Vec::new();
    }

    state.profile.update = match request {
        crate::state::ProfileUpdateRequest::SetDisplayName { display_name } => {
            crate::state::ProfileUpdateState::SettingDisplayName {
                request_id,
                display_name,
            }
        }
        crate::state::ProfileUpdateRequest::SetAvatar {
            mime_type,
            byte_count,
        } => crate::state::ProfileUpdateState::SettingAvatar {
            request_id,
            mime_type,
            byte_count,
        },
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)]
}

pub(crate) fn handle_profile_update_succeeded(
    state: &mut AppState,
    request_id: u64,
    profile: crate::state::OwnProfile,
) -> Vec<AppEffect> {
    if state.profile.update.request_id() != Some(request_id) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    state.profile.update = crate::state::ProfileUpdateState::Idle;
    state.profile.own = profile;
    crate::state::refresh_profile_user_display_projection(
        &mut state.profile,
        own_user_id.as_deref(),
    );
    let room_members_changed =
        refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
    let room_list_changed =
        refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
    let native_attention_changed =
        room_list_changed && refresh_native_attention_candidate_display_projection(state);
    profile_changed_effects(
        room_members_changed,
        room_list_changed,
        native_attention_changed,
    )
}

pub(crate) fn handle_profile_update_failed(
    state: &mut AppState,
    request_id: u64,
    message: String,
) -> Vec<AppEffect> {
    if state.profile.update.request_id() != Some(request_id) {
        return Vec::new();
    }

    state.profile.update = crate::state::ProfileUpdateState::Idle;
    state.errors.push(AppError {
        code: "profile_update_failed".to_owned(),
        message,
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::ProfileChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}
