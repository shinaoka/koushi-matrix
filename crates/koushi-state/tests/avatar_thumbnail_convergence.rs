use koushi_state::{
    AppAction, AppState, AvatarImage, AvatarThumbnailState, LiveEventReceipts, LiveReadReceipt,
    SessionInfo, SessionState, UserProfile, reduce,
};

const ROOM_ID: &str = "!room:example.invalid";
const USER_ID: &str = "@reader:example.invalid";
const AVATAR_A: &str = "mxc://example.invalid/avatar-a";
const AVATAR_B: &str = "mxc://example.invalid/avatar-b";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@me:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn avatar(mxc_uri: &str, thumbnail: AvatarThumbnailState) -> AvatarImage {
    AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail,
    }
}

fn profile(mxc_uri: &str) -> UserProfile {
    UserProfile {
        user_id: USER_ID.to_owned(),
        display_name: Some("Reader".to_owned()),
        display_label: "Reader".to_owned(),
        original_display_label: "Reader".to_owned(),
        mention_search_terms: Vec::new(),
        avatar: Some(avatar(mxc_uri, AvatarThumbnailState::NotRequested)),
    }
}

fn ready_thumbnail() -> AvatarThumbnailState {
    AvatarThumbnailState::Ready {
        source_ref: "avatar/synthetic".to_owned(),
        width: Some(18),
        height: Some(18),
        mime_type: Some("image/png".to_owned()),
    }
}

fn receipt(event_id: &str) -> LiveEventReceipts {
    LiveEventReceipts {
        event_id: event_id.to_owned(),
        receipts: vec![LiveReadReceipt {
            user_id: USER_ID.to_owned(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(1),
        }],
    }
}

#[test]
fn room_observation_and_receipt_prefer_authoritative_matching_ready_thumbnail() {
    let mut state = ready_state();
    let ready = ready_thumbnail();
    state
        .profile
        .room_users
        .entry(ROOM_ID.to_owned())
        .or_default()
        .insert(USER_ID.to_owned(), {
            let mut profile = profile(AVATAR_A);
            profile.avatar = Some(avatar(AVATAR_A, ready.clone()));
            profile
        });

    reduce(
        &mut state,
        AppAction::LiveRoomProfilesObserved {
            room_id: ROOM_ID.to_owned(),
            profiles: vec![profile(AVATAR_A)],
        },
    );
    assert_eq!(
        state.profile.room_users[ROOM_ID][USER_ID]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&ready)
    );

    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsUpdated {
            room_id: ROOM_ID.to_owned(),
            receipts_by_event: vec![LiveEventReceipts {
                event_id: "$event:example.invalid".to_owned(),
                receipts: vec![LiveReadReceipt {
                    user_id: USER_ID.to_owned(),
                    display_name: None,
                    original_display_label: String::new(),
                    avatar: Some(avatar(AVATAR_A, AvatarThumbnailState::NotRequested)),
                    timestamp_ms: Some(1),
                }],
            }],
        },
    );
    assert_eq!(
        state.live_signals.rooms[ROOM_ID].receipts_by_event["$event:example.invalid"].readers[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&ready)
    );
}

#[test]
fn receipt_window_reuses_ready_thumbnail_from_existing_receipt_state() {
    let mut state = ready_state();
    let ready = ready_thumbnail();
    state.live_signals.rooms.insert(
        ROOM_ID.to_owned(),
        koushi_state::RoomLiveSignals {
            receipts_by_event: [(
                "$old:example.invalid".to_owned(),
                koushi_state::LiveEventReceiptSummary {
                    readers: vec![LiveReadReceipt {
                        user_id: USER_ID.to_owned(),
                        display_name: Some("Reader".to_owned()),
                        original_display_label: "Reader".to_owned(),
                        avatar: Some(avatar(AVATAR_A, ready.clone())),
                        timestamp_ms: Some(1),
                    }],
                    total_count: 1,
                    overflow_count: 0,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: ROOM_ID.to_owned(),
            scoped_event_ids: Vec::new(),
            receipts_by_event: vec![LiveEventReceipts {
                event_id: "$new:example.invalid".to_owned(),
                receipts: vec![LiveReadReceipt {
                    user_id: USER_ID.to_owned(),
                    display_name: None,
                    original_display_label: String::new(),
                    avatar: Some(avatar(AVATAR_A, AvatarThumbnailState::NotRequested)),
                    timestamp_ms: Some(1),
                }],
            }],
        },
    );
    assert_eq!(
        state.live_signals.rooms[ROOM_ID].receipts_by_event["$new:example.invalid"].readers[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&ready)
    );
}

#[test]
fn profile_and_receipt_observations_preserve_matching_ready_and_reset_changed_mxc() {
    let mut state = ready_state();
    let ready = ready_thumbnail();
    let mut seeded = profile(AVATAR_A);
    seeded.avatar = Some(avatar(AVATAR_A, ready.clone()));
    state
        .profile
        .users
        .insert(USER_ID.to_owned(), seeded.clone());
    state
        .profile
        .room_users
        .entry(ROOM_ID.to_owned())
        .or_default()
        .insert(USER_ID.to_owned(), seeded);

    reduce(
        &mut state,
        AppAction::LiveRoomProfilesObserved {
            room_id: ROOM_ID.to_owned(),
            profiles: vec![profile(AVATAR_A)],
        },
    );
    assert_eq!(
        state.profile.room_users[ROOM_ID][USER_ID]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&ready)
    );

    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![profile(AVATAR_A)],
        },
    );
    assert_eq!(
        state.profile.users[USER_ID]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&ready)
    );

    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsUpdated {
            room_id: ROOM_ID.to_owned(),
            receipts_by_event: vec![receipt("$event:example.invalid")],
        },
    );
    assert_eq!(
        state.live_signals.rooms[ROOM_ID].receipts_by_event["$event:example.invalid"].readers[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&ready)
    );

    reduce(
        &mut state,
        AppAction::LiveRoomProfilesObserved {
            room_id: ROOM_ID.to_owned(),
            profiles: vec![profile(AVATAR_B)],
        },
    );
    assert_eq!(
        state.profile.room_users[ROOM_ID][USER_ID]
            .avatar
            .as_ref()
            .map(|avatar| (&avatar.mxc_uri, &avatar.thumbnail)),
        Some((&AVATAR_B.to_owned(), &AvatarThumbnailState::NotRequested))
    );

    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: ROOM_ID.to_owned(),
            scoped_event_ids: vec!["$event:example.invalid".to_owned()],
            receipts_by_event: vec![receipt("$event:example.invalid")],
        },
    );
    assert_eq!(
        state.live_signals.rooms[ROOM_ID].receipts_by_event["$event:example.invalid"].readers[0]
            .avatar
            .as_ref()
            .map(|avatar| (&avatar.mxc_uri, &avatar.thumbnail)),
        Some((&AVATAR_B.to_owned(), &AvatarThumbnailState::NotRequested))
    );
}
