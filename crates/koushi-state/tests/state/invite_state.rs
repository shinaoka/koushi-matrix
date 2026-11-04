use std::collections::BTreeSet;

use koushi_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState, InvitePreview,
    RoomListFilter, SessionInfo, SessionState, UiEvent, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:localhost".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn invite_preview(room_id: &str, is_dm: bool) -> InvitePreview {
    InvitePreview {
        room_id: room_id.to_owned(),
        display_name: "Invite preview".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: Some("Project room".to_owned()),
        inviter_display_name: Some("Inviter".to_owned()),
        inviter_user_id: Some("@inviter:localhost".to_owned()),
        is_dm,
        is_space: false,
    }
}

fn ready_avatar_thumbnail(label: &str) -> AvatarThumbnailState {
    AvatarThumbnailState::Ready {
        source_ref: format!("avatar/{:016x}", label.len()),
        width: Some(64),
        height: Some(64),
        mime_type: Some("image/png".to_owned()),
    }
}

fn invite_preview_with_avatar(room_id: &str, is_dm: bool, avatar: AvatarImage) -> InvitePreview {
    InvitePreview {
        avatar: Some(avatar),
        ..invite_preview(room_id, is_dm)
    }
}

#[test]
fn invite_list_is_rust_owned_and_replaces_by_snapshot() {
    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );

    assert_eq!(
        state.invites,
        vec![invite_preview("!room:localhost", false)]
    );
    assert_eq!(
        effects,
        vec![koushi_state::AppEffect::EmitUiEvent(
            UiEvent::RoomListChanged
        )]
    );

    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!dm:localhost", true)],
        },
    );
    assert_eq!(state.invites, vec![invite_preview("!dm:localhost", true)]);
}

#[test]
fn invite_list_requires_exact_ready_and_clears_on_logout() {
    let mut signed_out = AppState::default();
    reduce(
        &mut signed_out,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(signed_out.invites.is_empty());

    let mut locked = ready_state();
    locked.session = SessionState::Locked(SessionInfo {
        homeserver: "http://127.0.0.1:6167".to_owned(),
        user_id: "@qa:localhost".to_owned(),
        device_id: "LOCALDEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    });
    reduce(
        &mut locked,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(locked.invites.is_empty());

    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(!state.invites.is_empty());

    reduce(&mut state, AppAction::LogoutFinished);
    assert!(state.invites.is_empty());
}

/// Characterization: invites whose inviter is in the ignored-user set are
/// invisible in the Invites room-list filter. The raw `state.invites` still
/// holds the full list (Rust owns it), but the room_list projection omits the
/// invite from an ignored user. This guards the `visible_invites_for_ignored_users`
/// helper path in the reducer.
#[test]
fn invite_list_update_is_whole_state_inert_in_locked_and_switching_contexts() {
    for session in [
        SessionState::Locked(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:localhost".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        SessionState::SwitchingAccount {
            info: SessionInfo {
                homeserver: "http://127.0.0.1:6167".to_owned(),
                user_id: "@qa:localhost".to_owned(),
                device_id: "LOCALDEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        },
    ] {
        let mut state = ready_state();
        state.session = session;
        state.invites = vec![invite_preview("!existing:localhost", false)];
        let before = state.clone();

        assert!(
            reduce(
                &mut state,
                AppAction::InviteListUpdated {
                    invites: vec![invite_preview("!incoming:localhost", true)],
                },
            )
            .is_empty()
        );
        assert_eq!(state, before);
    }
}

#[test]
fn invite_list_filters_invites_from_ignored_inviters_in_room_list_projection() {
    let mut state = ready_state();

    // Load ignored users before the invite list arrives.
    let mut ignored: BTreeSet<String> = BTreeSet::new();
    ignored.insert("@blocked:localhost".to_owned());
    reduce(
        &mut state,
        AppAction::IgnoredUsersLoaded { user_ids: ignored },
    );

    // Switch room list to Invites filter so the projection is active.
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Invites,
        },
    );

    // Add two invites: one from the ignored inviter, one from a normal inviter.
    let invite_from_ignored = InvitePreview {
        room_id: "!blocked-room:localhost".to_owned(),
        display_name: "Blocked Room".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        inviter_display_name: Some("Blocked".to_owned()),
        inviter_user_id: Some("@blocked:localhost".to_owned()),
        is_dm: false,
        is_space: false,
    };
    let invite_from_normal = InvitePreview {
        room_id: "!normal-room:localhost".to_owned(),
        display_name: "Normal Room".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        inviter_display_name: Some("Normal".to_owned()),
        inviter_user_id: Some("@normal:localhost".to_owned()),
        is_dm: false,
        is_space: false,
    };
    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_from_ignored.clone(), invite_from_normal.clone()],
        },
    );

    // The raw invite list is still complete (Rust owns it; React must not filter locally).
    assert_eq!(state.invites.len(), 2, "raw invite list must be complete");

    // The room_list projection must suppress the ignored-inviter invite.
    let projected_ids: Vec<&str> = state
        .room_list
        .items
        .iter()
        .map(|item| item.room_id.as_str())
        .collect();
    assert!(
        !projected_ids.contains(&"!blocked-room:localhost"),
        "invite from ignored inviter must not appear in room_list projection"
    );
    assert!(
        projected_ids.contains(&"!normal-room:localhost"),
        "invite from normal inviter must appear in room_list projection"
    );
}

/// Characterization: `InviteListUpdated` replaces the entire list — it is
/// a snapshot, not a delta. Sending an empty list clears all previous invites.
/// This guards the "no React-local invite accumulation" rule.
#[test]
fn invite_list_updated_replaces_entire_list_not_delta() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![
                invite_preview("!room-a:localhost", false),
                invite_preview("!room-b:localhost", false),
            ],
        },
    );
    assert_eq!(state.invites.len(), 2);

    // Replace with a single-item snapshot — the previous two must be gone.
    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room-c:localhost", true)],
        },
    );
    assert_eq!(state.invites.len(), 1);
    assert_eq!(state.invites[0].room_id, "!room-c:localhost");

    // Empty snapshot clears everything.
    reduce(&mut state, AppAction::InviteListUpdated { invites: vec![] });
    assert!(
        state.invites.is_empty(),
        "empty snapshot must clear all invites"
    );
}

/// Characterization: account switch clears invite list as part of the
/// session teardown. This guards the rule that invite state is Rust-owned and
/// isolated per session — a switch must not carry over invites from the old account.
#[test]
fn invite_list_is_cleared_on_account_switch() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(!state.invites.is_empty());

    let alternate_info = SessionInfo {
        homeserver: "http://127.0.0.1:6167".to_owned(),
        user_id: "@other:localhost".to_owned(),
        device_id: "OTHERDEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: alternate_info,
        },
    );

    assert!(
        state.invites.is_empty(),
        "invite list must be cleared on account switch"
    );
}

#[test]
fn invite_list_updated_preserves_downloaded_avatar_thumbnails() {
    let mut state = ready_state();
    let mxc_uri = "mxc://example.invalid/invite-avatar";
    let thumbnail = ready_avatar_thumbnail("invite");
    state.invites = vec![invite_preview_with_avatar(
        "!invite-old:localhost",
        false,
        AvatarImage {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: thumbnail.clone(),
        },
    )];

    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview_with_avatar(
                "!invite-new:localhost",
                false,
                AvatarImage {
                    mxc_uri: mxc_uri.to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                },
            )],
        },
    );

    assert_eq!(
        state.invites[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
}

#[test]
fn invite_list_updated_preserves_avatar_thumbnails_from_room_snapshot_state() {
    let mut state = ready_state();
    let mxc_uri = "mxc://example.invalid/shared-avatar";
    let thumbnail = ready_avatar_thumbnail("shared");
    state.rooms = vec![koushi_state::RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: "!dm-avatar:localhost".to_owned(),
        display_name: "Avatar Room".to_owned(),
        display_label: "Avatar Room".to_owned(),
        original_display_label: "Avatar Room".to_owned(),
        avatar: Some(AvatarImage {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: thumbnail.clone(),
        }),
        is_dm: true,
        dm_user_ids: vec!["@shared:localhost".to_owned()],
        tags: koushi_state::RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: vec![],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 0,
    }];

    reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview_with_avatar(
                "!invite-shared:localhost",
                false,
                AvatarImage {
                    mxc_uri: mxc_uri.to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                },
            )],
        },
    );

    assert_eq!(
        state.invites[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
}

/// Characterization: `InviteListUpdated` emits `RoomListChanged` in all cases
/// (empty list, non-empty list). This pins the effect contract so React always
/// gets a signal to re-render the invite badge count.
#[test]
fn invite_list_update_always_emits_room_list_changed_effect() {
    let mut state = ready_state();

    let effects = reduce(&mut state, AppAction::InviteListUpdated { invites: vec![] });
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)),
        "empty invite update must still emit RoomListChanged"
    );

    let effects = reduce(
        &mut state,
        AppAction::InviteListUpdated {
            invites: vec![invite_preview("!room:localhost", false)],
        },
    );
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)),
        "non-empty invite update must emit RoomListChanged"
    );
}

#[test]
fn provisional_snapshot_retains_an_invite_it_did_not_observe() {
    // #1737: a restored cache projection can omit a cached invite because the
    // current observation only covers what the cache holds. It refreshes known
    // invites and keeps the unobserved ones; only an authoritative snapshot may
    // retract an invite.
    let mut state = ready_state();
    state.room_list.readiness = koushi_state::RoomListReadiness::Loading {
        source: koushi_state::RoomListSource::Live,
        generation: 4,
    };
    state.invites = vec![
        invite_preview("!kept:localhost", false),
        invite_preview("!refreshed:localhost", true),
    ];

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 4,
            source: koushi_state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: vec![InvitePreview {
                display_name: "Invite preview updated".to_owned(),
                ..invite_preview("!refreshed:localhost", true)
            }],
        },
    );

    assert_eq!(state.invites.len(), 2);
    assert!(
        state
            .invites
            .iter()
            .any(|invite| invite.room_id == "!kept:localhost")
    );
    assert!(
        state
            .invites
            .iter()
            .any(|invite| invite.room_id == "!refreshed:localhost"
                && invite.display_name == "Invite preview updated")
    );

    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 4,
            source: koushi_state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );
    assert!(
        state.invites.is_empty(),
        "an authoritative snapshot retracts every invite it did not observe"
    );
}

#[test]
fn a_locally_declined_invite_is_not_restored_by_a_provisional_snapshot() {
    // #1737: the provisional merge retains invites it did not observe, so the
    // local decline has to be recorded or the invite would come back on the next
    // cache projection.
    let mut state = ready_state();
    state.room_list.readiness = koushi_state::RoomListReadiness::Loading {
        source: koushi_state::RoomListSource::Live,
        generation: 5,
    };
    state.invites = vec![invite_preview("!declined:localhost", false)];

    let effects = reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "!declined:localhost".to_owned(),
        },
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert!(state.invites.is_empty());

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 5,
            source: koushi_state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: vec![
                invite_preview("!declined:localhost", false),
                invite_preview("!other:localhost", false),
            ],
        },
    );

    assert_eq!(state.invites.len(), 1);
    assert_eq!(state.invites[0].room_id, "!other:localhost");
}

#[test]
fn room_list_snapshots_keep_an_invite_avatar_thumbnail_only_for_the_same_face() {
    // #1773: both snapshot handlers carry a normalized invite whose thumbnail is
    // NotRequested. A previously downloaded Ready thumbnail must survive the
    // snapshot, but only while the same room still shows the same MXC URI.
    let avatar_mxc = "mxc://localhost/invite-avatar";
    let changed_mxc = "mxc://localhost/invite-avatar-changed";
    let expected_thumbnail = ready_avatar_thumbnail("invite");

    for snapshot_kind in ["provisional", "authoritative"] {
        let mut state = ready_state();
        state.room_list.readiness = koushi_state::RoomListReadiness::Loading {
            source: koushi_state::RoomListSource::Live,
            generation: 7,
        };
        state.invites = vec![invite_preview_with_avatar(
            "!invite:localhost",
            false,
            AvatarImage {
                mxc_uri: avatar_mxc.to_owned(),
                thumbnail: expected_thumbnail.clone(),
            },
        )];
        let snapshot = |mxc_uri: &str| {
            let invites = vec![invite_preview_with_avatar(
                "!invite:localhost",
                false,
                AvatarImage {
                    mxc_uri: mxc_uri.to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                },
            )];
            if snapshot_kind == "provisional" {
                AppAction::RoomListSnapshotProvisional {
                    generation: 7,
                    source: koushi_state::RoomListSource::Live,
                    spaces: Vec::new(),
                    rooms: Vec::new(),
                    invites,
                }
            } else {
                AppAction::RoomListSnapshotAuthoritative {
                    generation: 7,
                    source: koushi_state::RoomListSource::Live,
                    spaces: Vec::new(),
                    rooms: Vec::new(),
                    invites,
                }
            }
        };

        reduce(&mut state, snapshot(avatar_mxc));
        assert_eq!(
            state.invites[0]
                .avatar
                .as_ref()
                .map(|avatar| avatar.thumbnail.clone()),
            Some(expected_thumbnail.clone()),
            "{snapshot_kind}: the matching MXC URI keeps the complete Ready thumbnail"
        );

        reduce(&mut state, snapshot(changed_mxc));
        assert_eq!(
            state.invites[0]
                .avatar
                .as_ref()
                .map(|avatar| &avatar.thumbnail),
            Some(&AvatarThumbnailState::NotRequested),
            "{snapshot_kind}: a changed MXC URI must not inherit the old face"
        );
    }
}

#[test]
fn invite_snapshot_preserves_in_flight_state_only_for_the_same_room_and_face() {
    let avatar_mxc = "mxc://localhost/shared-invite-avatar";
    let loading = AvatarThumbnailState::Loading { request_id: 11 };

    for snapshot_kind in ["provisional", "authoritative"] {
        let snapshot = |room_id: &str| {
            let invites = vec![invite_preview_with_avatar(
                room_id,
                false,
                AvatarImage {
                    mxc_uri: avatar_mxc.to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                },
            )];
            if snapshot_kind == "provisional" {
                AppAction::RoomListSnapshotProvisional {
                    generation: 9,
                    source: koushi_state::RoomListSource::Live,
                    spaces: Vec::new(),
                    rooms: Vec::new(),
                    invites,
                }
            } else {
                AppAction::RoomListSnapshotAuthoritative {
                    generation: 9,
                    source: koushi_state::RoomListSource::Live,
                    spaces: Vec::new(),
                    rooms: Vec::new(),
                    invites,
                }
            }
        };

        let mut state = ready_state();
        state.room_list.readiness = koushi_state::RoomListReadiness::Loading {
            source: koushi_state::RoomListSource::Live,
            generation: 9,
        };
        state.invites = vec![invite_preview_with_avatar(
            "!invite:localhost",
            false,
            AvatarImage {
                mxc_uri: avatar_mxc.to_owned(),
                thumbnail: loading.clone(),
            },
        )];
        reduce(&mut state, snapshot("!invite:localhost"));
        assert_eq!(
            state.invites[0]
                .avatar
                .as_ref()
                .map(|avatar| avatar.thumbnail.clone()),
            Some(loading.clone()),
            "{snapshot_kind}: in-flight thumbnail survives the same face"
        );

        let mut state = ready_state();
        state.room_list.readiness = koushi_state::RoomListReadiness::Loading {
            source: koushi_state::RoomListSource::Live,
            generation: 9,
        };
        state.invites = vec![invite_preview_with_avatar(
            "!invite:localhost",
            false,
            AvatarImage {
                mxc_uri: avatar_mxc.to_owned(),
                thumbnail: ready_avatar_thumbnail("invite"),
            },
        )];
        reduce(&mut state, snapshot("!other-invite:localhost"));
        let other = state
            .invites
            .iter()
            .find(|invite| invite.room_id == "!other-invite:localhost")
            .expect("incoming invite");
        assert_eq!(
            other.avatar.as_ref().map(|avatar| &avatar.thumbnail),
            Some(&AvatarThumbnailState::NotRequested),
            "{snapshot_kind}: another room must not inherit the same MXC's thumbnail"
        );
    }
}
