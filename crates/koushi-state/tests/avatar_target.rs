use koushi_state::{
    AppState, AvatarImage, AvatarTarget, AvatarThumbnailState, UserProfile, resolve_avatar_target,
};

#[test]
fn room_profile_absence_is_authoritative_and_does_not_restore_a_global_avatar() {
    let mut state = AppState::default();
    let profile = UserProfile {
        user_id: "@synthetic:example.invalid".into(),
        display_name: None,
        display_label: String::new(),
        original_display_label: String::new(),
        mention_search_terms: vec![],
        avatar: Some(AvatarImage {
            mxc_uri: "mxc://example.invalid/global".into(),
            thumbnail: AvatarThumbnailState::NotRequested,
        }),
    };
    state
        .profile
        .users
        .insert(profile.user_id.clone(), profile.clone());
    let target = AvatarTarget::User {
        room_id: "!synthetic:example.invalid".into(),
        user_id: profile.user_id.clone(),
    };
    assert_eq!(
        resolve_avatar_target(&state, &target),
        Some("mxc://example.invalid/global")
    );
    let mut room_profile = profile;
    room_profile.avatar = None;
    state
        .profile
        .room_users
        .entry("!synthetic:example.invalid".into())
        .or_default()
        .insert(room_profile.user_id.clone(), room_profile);
    assert_eq!(resolve_avatar_target(&state, &target), None);
    assert!(!format!("{target:?}").contains("example.invalid"));
}

#[test]
fn entity_resolution_uses_only_existing_projected_identity() {
    let mut state = AppState::default();
    state.spaces.push(koushi_state::SpaceSummary {
        space_id: "!space:example.invalid".into(),
        display_name: "Synthetic".into(),
        child_room_ids: vec![],
        avatar: Some(AvatarImage {
            mxc_uri: "mxc://example.invalid/space".into(),
            thumbnail: AvatarThumbnailState::NotRequested,
        }),
    });
    let target = AvatarTarget::Space {
        space_id: "!space:example.invalid".into(),
    };
    assert_eq!(
        resolve_avatar_target(&state, &target),
        Some("mxc://example.invalid/space")
    );
    assert_eq!(
        resolve_avatar_target(
            &state,
            &AvatarTarget::Room {
                room_id: "!space:example.invalid".into()
            }
        ),
        None
    );
    assert_eq!(
        resolve_avatar_target(
            &state,
            &AvatarTarget::Invite {
                room_id: "!unknown:example.invalid".into()
            }
        ),
        None
    );
    assert_eq!(
        resolve_avatar_target(&state, &AvatarTarget::OwnProfile),
        None
    );
}
