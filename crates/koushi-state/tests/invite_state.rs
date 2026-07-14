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
        }),
        ..AppState::default()
    }
}

fn invite_preview(room_id: &str, is_dm: bool) -> InvitePreview {
    InvitePreview {
        room_id: room_id.to_owned(),
        display_name: "Invite preview".to_owned(),
        avatar: None,
        topic: Some("Project room".to_owned()),
        inviter_display_name: Some("Inviter".to_owned()),
        inviter_user_id: Some("@inviter:localhost".to_owned()),
        is_dm,
    }
}

fn ready_avatar_thumbnail(label: &str) -> AvatarThumbnailState {
    AvatarThumbnailState::Ready {
        source_url: format!("file:///tmp/koushi-test-{label}.png"),
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
        avatar: None,
        topic: None,
        inviter_display_name: Some("Blocked".to_owned()),
        inviter_user_id: Some("@blocked:localhost".to_owned()),
        is_dm: false,
    };
    let invite_from_normal = InvitePreview {
        room_id: "!normal-room:localhost".to_owned(),
        display_name: "Normal Room".to_owned(),
        avatar: None,
        topic: None,
        inviter_display_name: Some("Normal".to_owned()),
        inviter_user_id: Some("@normal:localhost".to_owned()),
        is_dm: false,
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
