use koushi_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState, InvitePreview,
    LiveEventReceipts, LiveReadReceipt, LocalUserAliasUpdateState, OwnProfile,
    ProfileUpdateRequest, ProfileUpdateState, RoomSummary, RoomTags, SessionInfo, SessionState,
    SpaceSummary, UiEvent, UserProfile, reduce, resolve_user_display_name,
};
use std::collections::BTreeMap;

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
                    display_label: String::new(),
                    original_display_label: String::new(),
                    mention_search_terms: Vec::new(),
                    avatar: Some(avatar("mxc://localhost/bob-avatar")),
                },
                UserProfile {
                    user_id: "@alice:localhost".to_owned(),
                    display_name: Some("Alice".to_owned()),
                    display_label: String::new(),
                    original_display_label: String::new(),
                    mention_search_terms: Vec::new(),
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
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
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
fn local_user_aliases_load_set_clear_and_settle_with_request_correlation() {
    let mut signed_out = AppState::default();
    reduce(
        &mut signed_out,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([("@bob:localhost".to_owned(), "Bobby".to_owned())]),
        },
    );
    assert!(signed_out.profile.local_aliases.is_empty());

    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([("@bob:localhost".to_owned(), "Bobby".to_owned())]),
        },
    );
    assert_eq!(
        state
            .profile
            .local_aliases
            .get("@bob:localhost")
            .map(String::as_str),
        Some("Bobby")
    );
    assert_eq!(effects, profile_changed());

    let effects = reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 7,
            user_id: "@bob:localhost".to_owned(),
            alias: Some("  Robert  ".to_owned()),
        },
    );
    assert_eq!(
        state
            .profile
            .local_aliases
            .get("@bob:localhost")
            .map(String::as_str),
        Some("Robert")
    );
    assert_eq!(
        state.profile.local_alias_update,
        LocalUserAliasUpdateState::Saving { request_id: 7 }
    );
    assert_eq!(effects, profile_changed());

    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 8,
            user_id: "@alice:localhost".to_owned(),
            alias: Some("Alice Alias".to_owned()),
        },
    );
    assert!(!state.profile.local_aliases.contains_key("@alice:localhost"));

    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateFailed {
            request_id: 99,
            message: "network".to_owned(),
        },
    );
    assert_eq!(
        state.profile.local_alias_update,
        LocalUserAliasUpdateState::Saving { request_id: 7 }
    );

    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateSucceeded { request_id: 7 },
    );
    assert_eq!(
        state.profile.local_alias_update,
        LocalUserAliasUpdateState::Idle
    );

    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 9,
            user_id: "@bob:localhost".to_owned(),
            alias: Some(" ".to_owned()),
        },
    );
    assert!(!state.profile.local_aliases.contains_key("@bob:localhost"));
}

#[test]
fn local_user_aliases_take_precedence_in_display_name_resolution() {
    let mut state = ready_state();
    state.profile.users.insert(
        "@bob:localhost".to_owned(),
        UserProfile {
            user_id: "@bob:localhost".to_owned(),
            display_name: Some("Bob".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([("@bob:localhost".to_owned(), "Bobby".to_owned())]),
        },
    );

    assert_eq!(
        resolve_user_display_name(
            &state.profile,
            "@bob:localhost",
            Some("Robert"),
            Some("@qa:localhost")
        ),
        "Bobby"
    );
    assert_eq!(
        resolve_user_display_name(
            &state.profile,
            "@alice:localhost",
            Some("Alice"),
            Some("@qa:localhost")
        ),
        "Alice"
    );
    assert_eq!(
        resolve_user_display_name(
            &state.profile,
            "@carol:localhost",
            None,
            Some("@qa:localhost")
        ),
        "@carol:localhost"
    );
}

#[test]
fn local_user_aliases_project_profile_display_labels_and_mention_search_terms() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![
                UserProfile {
                    user_id: "@bob:localhost".to_owned(),
                    display_name: Some("Bob Upstream".to_owned()),
                    display_label: String::new(),
                    original_display_label: String::new(),
                    mention_search_terms: Vec::new(),
                    avatar: None,
                },
                UserProfile {
                    user_id: "@carol:localhost".to_owned(),
                    display_name: None,
                    display_label: String::new(),
                    original_display_label: String::new(),
                    mention_search_terms: Vec::new(),
                    avatar: None,
                },
            ],
        },
    );
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([("@bob:localhost".to_owned(), "Bobby Local".to_owned())]),
        },
    );

    assert_eq!(
        state
            .profile
            .users
            .get("@bob:localhost")
            .map(|profile| profile.display_label.as_str()),
        Some("Bobby Local")
    );
    assert_eq!(
        state
            .profile
            .users
            .get("@carol:localhost")
            .map(|profile| profile.display_label.as_str()),
        Some("@carol:localhost")
    );
    assert_eq!(
        state
            .profile
            .users
            .get("@bob:localhost")
            .map(|profile| profile.original_display_label.as_str()),
        Some("Bob Upstream")
    );
    assert_eq!(
        state
            .profile
            .users
            .get("@carol:localhost")
            .map(|profile| profile.original_display_label.as_str()),
        Some("@carol:localhost")
    );
    assert_eq!(
        state
            .profile
            .users
            .get("@bob:localhost")
            .map(|profile| profile.mention_search_terms.clone())
            .unwrap_or_default(),
        vec![
            "Bobby Local".to_owned(),
            "Bob Upstream".to_owned(),
            "@bob:localhost".to_owned(),
        ]
    );

    reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 22,
            user_id: "@bob:localhost".to_owned(),
            alias: None,
        },
    );

    assert_eq!(
        state
            .profile
            .users
            .get("@bob:localhost")
            .map(|profile| profile.display_label.as_str()),
        Some("Bob Upstream")
    );
}

#[test]
fn local_user_aliases_project_receipt_original_display_labels() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![UserProfile {
                user_id: "@bob:localhost".to_owned(),
                display_name: Some("Bob Upstream".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            }],
        },
    );
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([("@bob:localhost".to_owned(), "Bobby".to_owned())]),
        },
    );

    let signals = koushi_state::LiveRoomSignalUpdate {
        receipts_by_event: vec![LiveEventReceipts {
            event_id: "$event:localhost".to_owned(),
            receipts: vec![LiveReadReceipt {
                user_id: "@bob:localhost".to_owned(),
                display_name: Some("Bob Room".to_owned()),
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(200),
            }],
        }],
        fully_read_event_id: None,
        typing_user_ids: Vec::new(),
    }
    .into_room_signals_with_profiles(&state.profile, Some("@qa:localhost"));
    let summary = signals
        .receipts_by_event
        .get("$event:localhost")
        .expect("receipt summary");

    assert_eq!(summary.readers[0].display_name.as_deref(), Some("Bobby"));
    assert_eq!(summary.readers[0].original_display_label, "Bob Room");
}

#[test]
fn local_user_aliases_debug_redacts_user_ids_and_aliases() {
    let mut profile = koushi_state::ProfileState::default();
    profile.own = OwnProfile {
        display_name: Some("Visible Own Name".to_owned()),
        avatar: Some(AvatarImage {
            mxc_uri: "mxc://example.invalid/own-avatar".to_owned(),
            thumbnail: AvatarThumbnailState::NotRequested,
        }),
    };
    profile.users.insert(
        "@carol:localhost".to_owned(),
        UserProfile {
            user_id: "@carol:localhost".to_owned(),
            display_name: Some("Visible Carol".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: Some(AvatarImage {
                mxc_uri: "mxc://example.invalid/carol-avatar".to_owned(),
                thumbnail: AvatarThumbnailState::NotRequested,
            }),
        },
    );
    profile
        .local_aliases
        .insert("@bob:localhost".to_owned(), "Bobby Private".to_owned());

    let debug = format!("{profile:?}");

    assert!(debug.contains("ProfileState"));
    assert!(debug.contains("user_count"));
    assert!(debug.contains("local_alias_count"));
    assert!(!debug.contains("Visible Own Name"));
    assert!(!debug.contains("Visible Carol"));
    assert!(!debug.contains("mxc://example.invalid"));
    assert!(!debug.contains("@carol:localhost"));
    assert!(!debug.contains("@bob:localhost"));
    assert!(!debug.contains("Bobby Private"));
}

#[test]
fn user_profile_debug_redacts_person_and_avatar_values() {
    let profile = UserProfile {
        user_id: "@carol:localhost".to_owned(),
        display_name: Some("Visible Carol".to_owned()),
        display_label: "Private Carol".to_owned(),
        original_display_label: "Private Carol".to_owned(),
        mention_search_terms: vec![
            "Private Carol".to_owned(),
            "Visible Carol".to_owned(),
            "@carol:localhost".to_owned(),
        ],
        avatar: Some(AvatarImage {
            mxc_uri: "mxc://example.invalid/carol-avatar".to_owned(),
            thumbnail: AvatarThumbnailState::Ready {
                source_url: "data:image/png;base64,secret".to_owned(),
                width: None,
                height: None,
                mime_type: Some("image/png".to_owned()),
            },
        }),
    };

    let debug = format!("{profile:?}");

    assert!(debug.contains("UserProfile"));
    assert!(debug.contains("has_avatar"));
    assert!(debug.contains("mention_search_terms"));
    assert!(!debug.contains("@carol:localhost"));
    assert!(!debug.contains("Visible Carol"));
    assert!(!debug.contains("Private Carol"));
    assert!(!debug.contains("mxc://example.invalid"));
    assert!(!debug.contains("data:image/png"));
}

#[test]
fn local_user_aliases_override_read_receipt_reader_labels() {
    let mut state = ready_state();
    state.profile.users.insert(
        "@bob:localhost".to_owned(),
        UserProfile {
            user_id: "@bob:localhost".to_owned(),
            display_name: Some("Bob".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([("@bob:localhost".to_owned(), "Bobby".to_owned())]),
        },
    );

    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsUpdated {
            room_id: "!room:localhost".to_owned(),
            receipts_by_event: vec![LiveEventReceipts {
                event_id: "$event:localhost".to_owned(),
                receipts: vec![LiveReadReceipt {
                    user_id: "@bob:localhost".to_owned(),
                    display_name: Some("Robert".to_owned()),
                    original_display_label: String::new(),
                    avatar: None,
                    timestamp_ms: Some(1),
                }],
            }],
        },
    );

    let summary = state.live_signals.rooms["!room:localhost"]
        .receipts_by_event
        .get("$event:localhost")
        .expect("receipt summary");
    assert_eq!(summary.readers[0].display_name.as_deref(), Some("Bobby"));
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
                display_label: "Room".to_owned(),
                original_display_label: "Room".to_owned(),
                avatar: Some(avatar("mxc://localhost/room-avatar")),
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                parent_space_ids: vec!["!space:localhost".to_owned()],
                is_encrypted: false,
                joined_members: 0,
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
                inviter_user_id: Some("@inviter:localhost".to_owned()),
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
fn ignored_users_load_filters_invites_and_presence() {
    let mut state = ready_state();
    state.invites = vec![InvitePreview {
        room_id: "!invite:localhost".to_owned(),
        display_name: "Invite".to_owned(),
        avatar: None,
        topic: None,
        inviter_display_name: Some("Inviter".to_owned()),
        inviter_user_id: Some("@ignored:localhost".to_owned()),
        is_dm: false,
    }];
    state.live_signals.presence.insert(
        "@ignored:localhost".to_owned(),
        koushi_state::PresenceKind::Online,
    );
    state.room_list.active_filter = koushi_state::RoomListFilter::Invites;

    let effects = reduce(
        &mut state,
        AppAction::IgnoredUsersLoaded {
            user_ids: ["@ignored:localhost".to_owned()].into_iter().collect(),
        },
    );

    assert!(
        state
            .profile
            .ignored_user_ids
            .contains("@ignored:localhost")
    );
    assert!(state.room_list.items.is_empty());
    assert!(state.live_signals.presence.is_empty());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ProfileChanged)))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::RoomListChanged)))
    );
}

#[test]
fn ignored_user_update_request_is_optimistic_and_sets_saving_state() {
    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::IgnoredUserUpdateRequested {
            request_id: 7,
            user_id: "@ignored:localhost".to_owned(),
            ignored: true,
        },
    );

    assert!(
        state
            .profile
            .ignored_user_ids
            .contains("@ignored:localhost")
    );
    assert_eq!(
        state.profile.ignored_user_update,
        koushi_state::IgnoredUserUpdateState::Saving { request_id: 7 }
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ProfileChanged)))
    );
}

#[test]
fn ignored_user_update_failed_reverts_optimistic_mutation() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::IgnoredUserUpdateRequested {
            request_id: 7,
            user_id: "@ignored:localhost".to_owned(),
            ignored: true,
        },
    );
    assert!(
        state
            .profile
            .ignored_user_ids
            .contains("@ignored:localhost")
    );

    let effects = reduce(
        &mut state,
        AppAction::IgnoredUserUpdateFailed {
            request_id: 7,
            user_id: "@ignored:localhost".to_owned(),
            ignored: true,
            message: "failed".to_owned(),
        },
    );

    assert!(
        !state
            .profile
            .ignored_user_ids
            .contains("@ignored:localhost")
    );
    assert!(state.profile.ignored_user_update.is_idle());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ProfileChanged)))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ErrorChanged)))
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
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
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
