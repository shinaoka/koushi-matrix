use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState, InvitePreview, OwnProfile,
    ProfileUpdateRequest, ProfileUpdateState, RoomSummary, RoomTags, SessionInfo, SessionState,
    SpaceSummary, UiEvent, UserProfile, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:localhost".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
        }),
        ..AppState::default()
    }
}

fn avatar(mxc_uri: &str) -> AvatarImage {
    AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

fn profile_changed() -> Vec<AppEffect> {
    vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)]
}

#[test]
fn own_profile_updates_are_rust_owned_and_require_ready_session() {
    let profile = OwnProfile {
        display_name: Some("QA User".to_owned()),
        avatar: Some(avatar("mxc://localhost/qa-avatar")),
    };

    let mut signed_out = AppState::default();
    reduce(
        &mut signed_out,
        AppAction::OwnProfileUpdated {
            profile: profile.clone(),
        },
    );
    assert_eq!(signed_out.profile.own, OwnProfile::default());

    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::OwnProfileUpdated {
            profile: profile.clone(),
        },
    );

    assert_eq!(state.profile.own, profile);
    assert_eq!(effects, profile_changed());
}

#[test]
fn user_profile_cache_is_replaced_by_rust_snapshot() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![
                UserProfile {
                    user_id: "@bob:localhost".to_owned(),
                    display_name: Some("Bob".to_owned()),
                    avatar: Some(avatar("mxc://localhost/bob-avatar")),
                },
                UserProfile {
                    user_id: "@alice:localhost".to_owned(),
                    display_name: Some("Alice".to_owned()),
                    avatar: None,
                },
            ],
        },
    );

    assert_eq!(
        state.profile.users.keys().cloned().collect::<Vec<_>>(),
        vec!["@alice:localhost".to_owned(), "@bob:localhost".to_owned()]
    );
    assert_eq!(
        state.profile.users["@bob:localhost"].avatar,
        Some(avatar("mxc://localhost/bob-avatar"))
    );

    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![UserProfile {
                user_id: "@carol:localhost".to_owned(),
                display_name: Some("Carol".to_owned()),
                avatar: None,
            }],
        },
    );
    assert_eq!(
        state.profile.users.keys().cloned().collect::<Vec<_>>(),
        vec!["@carol:localhost".to_owned()]
    );
}

#[test]
fn profile_update_state_is_request_correlated() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::ProfileUpdateRequested {
            request_id: 7,
            request: ProfileUpdateRequest::SetDisplayName {
                display_name: Some("New QA Name".to_owned()),
            },
        },
    );
    assert_eq!(
        state.profile.update,
        ProfileUpdateState::SettingDisplayName {
            request_id: 7,
            display_name: Some("New QA Name".to_owned()),
        }
    );

    reduce(
        &mut state,
        AppAction::ProfileUpdateRequested {
            request_id: 8,
            request: ProfileUpdateRequest::SetAvatar {
                mime_type: "image/png".to_owned(),
                byte_count: 42,
            },
        },
    );
    assert_eq!(
        state.profile.update,
        ProfileUpdateState::SettingDisplayName {
            request_id: 7,
            display_name: Some("New QA Name".to_owned()),
        }
    );

    reduce(
        &mut state,
        AppAction::ProfileUpdateSucceeded {
            request_id: 999,
            profile: OwnProfile {
                display_name: Some("Stale".to_owned()),
                avatar: None,
            },
        },
    );
    assert_eq!(
        state.profile.update,
        ProfileUpdateState::SettingDisplayName {
            request_id: 7,
            display_name: Some("New QA Name".to_owned()),
        }
    );

    reduce(
        &mut state,
        AppAction::ProfileUpdateSucceeded {
            request_id: 7,
            profile: OwnProfile {
                display_name: Some("New QA Name".to_owned()),
                avatar: Some(avatar("mxc://localhost/new-avatar")),
            },
        },
    );
    assert_eq!(state.profile.update, ProfileUpdateState::Idle);
    assert_eq!(
        state.profile.own,
        OwnProfile {
            display_name: Some("New QA Name".to_owned()),
            avatar: Some(avatar("mxc://localhost/new-avatar")),
        }
    );
}

#[test]
fn profile_update_failure_requires_matching_in_flight_request() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::ProfileUpdateFailed {
            request_id: 1,
            message: "network".to_owned(),
        },
    );
    assert_eq!(state.profile.update, ProfileUpdateState::Idle);
    assert!(state.errors.is_empty());

    reduce(
        &mut state,
        AppAction::ProfileUpdateRequested {
            request_id: 2,
            request: ProfileUpdateRequest::SetAvatar {
                mime_type: "image/jpeg".to_owned(),
                byte_count: 11,
            },
        },
    );
    reduce(
        &mut state,
        AppAction::ProfileUpdateFailed {
            request_id: 2,
            message: "network".to_owned(),
        },
    );

    assert_eq!(state.profile.update, ProfileUpdateState::Idle);
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "profile_update_failed");
}

#[test]
fn room_space_and_invite_summaries_surface_avatar_mxc() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![SpaceSummary {
                space_id: "!space:localhost".to_owned(),
                display_name: "Space".to_owned(),
                avatar: Some(avatar("mxc://localhost/space-avatar")),
                child_room_ids: vec!["!room:localhost".to_owned()],
            }],
            rooms: vec![RoomSummary {
                room_id: "!room:localhost".to_owned(),
                display_name: "Room".to_owned(),
                avatar: Some(avatar("mxc://localhost/room-avatar")),
                is_dm: false,
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                parent_space_ids: vec!["!space:localhost".to_owned()],
            }],
        },
    );
    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![InvitePreview {
                room_id: "!invite:localhost".to_owned(),
                display_name: "Invite".to_owned(),
                avatar: Some(avatar("mxc://localhost/invite-avatar")),
                topic: None,
                inviter_display_name: Some("Inviter".to_owned()),
                is_dm: false,
            }],
        },
    );

    assert_eq!(
        state.spaces[0].avatar,
        Some(avatar("mxc://localhost/space-avatar"))
    );
    assert_eq!(
        state.rooms[0].avatar,
        Some(avatar("mxc://localhost/room-avatar"))
    );
    assert_eq!(
        state.invites[0].avatar,
        Some(avatar("mxc://localhost/invite-avatar"))
    );
}

#[test]
fn profile_state_clears_with_session_views() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::OwnProfileUpdated {
            profile: OwnProfile {
                display_name: Some("QA User".to_owned()),
                avatar: Some(avatar("mxc://localhost/qa-avatar")),
            },
        },
    );
    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![UserProfile {
                user_id: "@bob:localhost".to_owned(),
                display_name: Some("Bob".to_owned()),
                avatar: Some(avatar("mxc://localhost/bob-avatar")),
            }],
        },
    );

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.profile.own, OwnProfile::default());
    assert!(state.profile.users.is_empty());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ProfileChanged)))
    );
}
