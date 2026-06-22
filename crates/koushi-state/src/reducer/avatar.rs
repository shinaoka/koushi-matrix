use std::collections::BTreeMap;

use crate::state::{AppState, AvatarImage, AvatarThumbnailState};

pub(crate) fn collect_known_avatar_thumbnails(
    state: &AppState,
    include_invites: bool,
) -> BTreeMap<String, AvatarThumbnailState> {
    let mut known_thumbnails = BTreeMap::new();
    remember_known_avatar_thumbnail(&mut known_thumbnails, state.profile.own.avatar.as_ref());
    for profile in state.profile.users.values() {
        remember_known_avatar_thumbnail(&mut known_thumbnails, profile.avatar.as_ref());
    }
    for room in &state.rooms {
        remember_known_avatar_thumbnail(&mut known_thumbnails, room.avatar.as_ref());
    }
    for space in &state.spaces {
        remember_known_avatar_thumbnail(&mut known_thumbnails, space.avatar.as_ref());
    }
    if include_invites {
        for invite in &state.invites {
            remember_known_avatar_thumbnail(&mut known_thumbnails, invite.avatar.as_ref());
        }
    }
    known_thumbnails
}

pub(crate) fn preserve_avatar_thumbnail(
    known_thumbnails: &BTreeMap<String, AvatarThumbnailState>,
    avatar: &mut Option<AvatarImage>,
) -> bool {
    let Some(avatar) = avatar.as_mut() else {
        return false;
    };
    if avatar.thumbnail != AvatarThumbnailState::NotRequested {
        return false;
    }
    let Some(thumbnail) = known_thumbnails.get(&avatar.mxc_uri) else {
        return false;
    };
    avatar.thumbnail = thumbnail.clone();
    true
}

fn remember_known_avatar_thumbnail(
    known_thumbnails: &mut BTreeMap<String, AvatarThumbnailState>,
    avatar: Option<&AvatarImage>,
) {
    let Some(avatar) = avatar else {
        return;
    };
    if avatar.thumbnail == AvatarThumbnailState::NotRequested {
        return;
    }
    known_thumbnails.insert(avatar.mxc_uri.clone(), avatar.thumbnail.clone());
}
