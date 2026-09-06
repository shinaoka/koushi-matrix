use super::*;
use crate::state::{
    AvatarImage, AvatarThumbnailState, LiveEventReceiptSummary, LiveEventReceipts, LiveReadReceipt,
    MediaTransferProgress, OperationFailureKind, PresenceKind, RoomLatestEventSummary,
    RoomLiveSignals, TimelineMediaDownloadState, UserProfile,
};

fn ready_state() -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(crate::state::SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: crate::state::SessionAuthenticationMethod::Unknown,
    });
    state
}

fn verification_gate(
    methods: Vec<crate::state::VerificationMethodCapability>,
) -> crate::state::VerificationGateState {
    crate::state::VerificationGateState {
        methods,
        account_kind: crate::state::VerificationAccountKind::ExistingIdentity,
        failure: None,
    }
}

fn awaiting_verification_state(
    methods: Vec<crate::state::VerificationMethodCapability>,
) -> AppState {
    let mut state = ready_state();
    let info = match state.session.clone() {
        SessionState::Ready(info) => info,
        other => panic!("ready_state must be Ready, got {other:?}"),
    };
    state.session = SessionState::AwaitingVerification {
        info,
        gate: verification_gate(methods),
    };
    state
}

fn stale_cancelled_verification() -> crate::state::VerificationFlowState {
    crate::state::VerificationFlowState::Failed {
        request_id: 9,
        target: crate::state::VerificationTarget {
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "OLD".to_owned(),
        },
        kind: crate::state::TrustOperationFailureKind::Cancelled,
    }
}

#[test]
fn verification_method_submission_clears_stale_device_verification_failure() {
    let mut state = awaiting_verification_state(vec![
        crate::state::VerificationMethodCapability::ExistingDeviceSas,
    ]);
    state.e2ee_trust.verification = stale_cancelled_verification();

    let effects = reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: crate::state::VerificationMethod::ExistingDeviceSas,
            flow_id: 10,
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SessionChanged)));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)));
    assert_eq!(
        state.e2ee_trust.verification,
        crate::state::VerificationFlowState::Idle
    );
}

#[test]
fn recovery_submission_clears_stale_device_verification_failure() {
    let mut state = awaiting_verification_state(vec![
        crate::state::VerificationMethodCapability::RecoveryKey,
    ]);
    state.e2ee_trust.verification = stale_cancelled_verification();

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoverySubmitted {
            flow_id: 11,
            request: crate::action::RecoveryRequest {
                secret: crate::action::AuthSecret::new("secret"),
            },
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SessionChanged)));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)));
    assert_eq!(
        state.e2ee_trust.verification,
        crate::state::VerificationFlowState::Idle
    );
}

fn test_space(space_id: &str) -> crate::state::SpaceSummary {
    crate::state::SpaceSummary {
        space_id: space_id.to_owned(),
        display_name: space_id.to_owned(),
        avatar: None,
        child_room_ids: Vec::new(),
    }
}

#[test]
fn space_order_preference_normalization_keeps_missing_ids_and_deduplicates() {
    let mut space_order = vec![
        "!space-a:example.invalid".to_owned(),
        "!space-b:example.invalid".to_owned(),
        "!space-a:example.invalid".to_owned(),
    ];

    normalize_space_order_preference(&mut space_order);

    assert_eq!(
        space_order,
        vec!["!space-a:example.invalid", "!space-b:example.invalid",]
    );
}

#[test]
fn reordering_visible_spaces_preserves_hidden_ledger_slots() {
    let current_spaces = vec![
        test_space("!space-a:example.invalid"),
        test_space("!space-c:example.invalid"),
    ];
    let mut space_order = vec![
        "!space-a:example.invalid".to_owned(),
        "!space-hidden:example.invalid".to_owned(),
        "!space-c:example.invalid".to_owned(),
    ];

    assert!(reorder_visible_space_order(
        &mut space_order,
        &current_spaces,
        &[
            "!space-c:example.invalid".to_owned(),
            "!space-a:example.invalid".to_owned(),
        ],
    ));

    assert_eq!(
        space_order,
        vec![
            "!space-c:example.invalid",
            "!space-hidden:example.invalid",
            "!space-a:example.invalid",
        ]
    );
}

#[test]
fn room_list_updates_do_not_drop_persisted_spaces_before_the_first_snapshot() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::NavigationLoaded {
            navigation: NavigationState {
                space_order: vec![
                    "!space-a:example.invalid".to_owned(),
                    "!space-b:example.invalid".to_owned(),
                ],
                ..NavigationState::default()
            },
        },
    );
    assert!(state.spaces.is_empty());

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![
                test_space("!space-b:example.invalid"),
                test_space("!space-c:example.invalid"),
            ],
            rooms: Vec::new(),
        },
    );

    assert_eq!(
        state.navigation.space_order,
        vec![
            "!space-a:example.invalid",
            "!space-b:example.invalid",
            "!space-c:example.invalid",
        ]
    );

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![
                test_space("!space-a:example.invalid"),
                test_space("!space-b:example.invalid"),
                test_space("!space-c:example.invalid"),
            ],
            rooms: Vec::new(),
        },
    );

    assert_eq!(
        state
            .spaces
            .iter()
            .map(|space| space.space_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "!space-a:example.invalid",
            "!space-b:example.invalid",
            "!space-c:example.invalid",
        ]
    );
}

#[test]
fn explicit_space_order_preference_removal_removes_only_the_requested_entry() {
    let mut state = ready_state();
    state.spaces = vec![
        test_space("!space-a:example.invalid"),
        test_space("!space-b:example.invalid"),
    ];
    state.navigation.space_order = vec![
        "!space-a:example.invalid".to_owned(),
        "!space-b:example.invalid".to_owned(),
    ];

    let effects = reduce(
        &mut state,
        AppAction::SpaceOrderPreferenceRemoved {
            space_id: "!space-b:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
    assert_eq!(
        state.navigation.space_order,
        vec!["!space-a:example.invalid"]
    );
    assert_eq!(state.spaces.len(), 2);
}

fn test_avatar(mxc_uri: &str) -> AvatarImage {
    AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

fn ready_avatar_thumbnail(label: &str) -> AvatarThumbnailState {
    AvatarThumbnailState::Ready {
        source_ref: format!("fixture-avatar-{label}"),
        width: Some(64),
        height: Some(64),
        mime_type: Some("image/png".to_owned()),
    }
}

fn test_room(room_id: &str, avatar: Option<AvatarImage>) -> crate::state::RoomSummary {
    crate::state::RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: crate::state::RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

#[test]
fn room_list_bootstrap_does_not_publish_unproven_empty_snapshot() {
    let mut state = ready_state();
    state.rooms = vec![test_room("!cached:example.invalid", None)];
    let cached_rooms = state.rooms.clone();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: cached_rooms,
        },
    );
    let cached_projection = state.room_list.clone();

    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 7,
            source: crate::state::RoomListSource::Live,
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 7,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.rooms,
        vec![test_room("!cached:example.invalid", None)]
    );
    assert_eq!(state.room_list.items, cached_projection.items);
    assert!(matches!(
        state.room_list.readiness,
        crate::state::RoomListReadiness::Loading { generation: 7, .. }
    ));
}

#[test]
fn room_list_bootstrap_does_not_clear_cached_rooms_for_invite_only_provisional_snapshot() {
    let mut state = ready_state();
    state.rooms = vec![test_room("!cached:example.invalid", None)];
    let cached_rooms = state.rooms.clone();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: cached_rooms,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 8,
            source: crate::state::RoomListSource::Live,
        },
    );

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 8,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: vec![crate::state::InvitePreview {
                room_id: "!invite:example.invalid".to_owned(),
                display_name: "Invite".to_owned(),
                avatar: None,
                topic: None,
                inviter_display_name: None,
                inviter_user_id: None,
                is_dm: false,
            }],
        },
    );

    assert_eq!(state.rooms[0].room_id, "!cached:example.invalid");
    assert_eq!(state.invites[0].room_id, "!invite:example.invalid");
}

#[test]
fn room_list_bootstrap_accepts_current_authoritative_zero_and_retains_failed_cache() {
    let mut state = ready_state();
    state.rooms = vec![test_room("!cached:example.invalid", None)];
    let cached_rooms = state.rooms.clone();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: cached_rooms,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 11,
            source: crate::state::RoomListSource::Live,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 11,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );
    assert!(state.rooms.is_empty());
    assert!(matches!(
        state.room_list.readiness,
        crate::state::RoomListReadiness::Ready { generation: 11, .. }
    ));

    state.rooms = vec![test_room("!cached-again:example.invalid", None)];
    let cached_rooms = state.rooms.clone();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: cached_rooms,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 12,
            source: crate::state::RoomListSource::Live,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListBootstrapFailed {
            generation: 12,
            source: crate::state::RoomListSource::Live,
            kind: crate::state::RoomListFailureKind::Connectivity,
        },
    );
    assert_eq!(
        state.rooms,
        vec![test_room("!cached-again:example.invalid", None)]
    );
    assert!(matches!(
        state.room_list.readiness,
        crate::state::RoomListReadiness::Failed { generation: 12, .. }
    ));
}

#[test]
fn room_list_bootstrap_ignores_retired_generation() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 3,
            source: crate::state::RoomListSource::Live,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 3,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room("!current:example.invalid", None)],
            invites: Vec::new(),
        },
    );
    let current_projection = state.room_list.clone();

    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 2,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room("!stale:example.invalid", None)],
            invites: Vec::new(),
        },
    );

    assert_eq!(state.rooms[0].room_id, "!current:example.invalid");
    assert_eq!(state.room_list, current_projection);
}

#[test]
fn search_crawler_is_admitted_only_after_authoritative_room_list_readiness() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 21,
            source: crate::state::RoomListSource::Live,
        },
    );
    let provisional_effects = reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 21,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room("!provisional:example.invalid", None)],
            invites: Vec::new(),
        },
    );
    assert!(
        !provisional_effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::NotifySearchCrawlerRoomsAvailable { .. }))
    );

    let authoritative_effects = reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 21,
            source: crate::state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room("!authoritative:example.invalid", None)],
            invites: Vec::new(),
        },
    );
    assert!(
        authoritative_effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::NotifySearchCrawlerRoomsAvailable { .. }))
    );
}

fn latest_event(event_id: &str, timestamp_ms: u64) -> RoomLatestEventSummary {
    RoomLatestEventSummary {
        event_id: event_id.to_owned(),
        relation_type: None,
        relation_event_id: None,
        sender_id: Some("@bob:example.invalid".to_owned()),
        sender_label: Some("Bob".to_owned()),
        sender_avatar: None,
        preview: Some("body".to_owned()),
        timestamp_ms,
        is_redacted: false,
    }
}

#[test]
fn avatar_thumbnail_updates_rust_owned_snapshots() {
    let mut state = ready_state();
    let mxc_uri = "mxc://example.invalid/avatar";
    state.profile.own.avatar = Some(test_avatar(mxc_uri));
    state.profile.users.insert(
        "@bob:example.invalid".to_owned(),
        UserProfile {
            user_id: "@bob:example.invalid".to_owned(),
            display_name: Some("Bob".to_owned()),
            display_label: "Bob".to_owned(),
            original_display_label: "Bob".to_owned(),
            mention_search_terms: Vec::new(),
            avatar: Some(test_avatar(mxc_uri)),
        },
    );
    state.rooms = vec![test_room(
        "!room:example.invalid",
        Some(test_avatar(mxc_uri)),
    )];
    state.spaces = vec![crate::state::SpaceSummary {
        avatar: Some(test_avatar(mxc_uri)),
        ..test_space("!space:example.invalid")
    }];
    state.invites = vec![crate::state::InvitePreview {
        room_id: "!invite:example.invalid".to_owned(),
        display_name: "Invite".to_owned(),
        avatar: Some(test_avatar(mxc_uri)),
        topic: None,
        inviter_display_name: None,
        inviter_user_id: None,
        is_dm: false,
    }];

    let thumbnail = ready_avatar_thumbnail("avatar");
    let effects = reduce(
        &mut state,
        AppAction::AvatarThumbnailUpdated {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: thumbnail.clone(),
        },
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ProfileChanged(_))))
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state
            .profile
            .own
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
    assert_eq!(
        state
            .profile
            .users
            .get("@bob:example.invalid")
            .and_then(|profile| profile.avatar.as_ref())
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
    assert_eq!(
        state.rooms[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
    assert_eq!(
        state.spaces[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
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
fn room_list_updates_preserve_downloaded_avatar_thumbnails() {
    let mut state = ready_state();
    let mxc_uri = "mxc://example.invalid/avatar";
    let thumbnail = ready_avatar_thumbnail("preserved");
    state.rooms = vec![test_room(
        "!room:example.invalid",
        Some(AvatarImage {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: thumbnail.clone(),
        }),
    )];

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![test_room(
                "!room:example.invalid",
                Some(test_avatar(mxc_uri)),
            )],
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state.rooms[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
}

#[test]
fn reorder_spaces_persists_and_reapplies_to_room_list_updates() {
    let mut state = ready_state();
    state.spaces = vec![
        test_space("!space-a:example.invalid"),
        test_space("!space-b:example.invalid"),
    ];

    let effects = reduce(
        &mut state,
        AppAction::ReorderSpaces {
            space_ids: vec![
                "!space-b:example.invalid".to_owned(),
                "!space-a:example.invalid".to_owned(),
            ],
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
    assert_eq!(
        state.navigation.space_order,
        vec!["!space-b:example.invalid", "!space-a:example.invalid"]
    );
    assert_eq!(
        state
            .spaces
            .iter()
            .map(|space| space.space_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!space-b:example.invalid", "!space-a:example.invalid"]
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![
                test_space("!space-a:example.invalid"),
                test_space("!space-b:example.invalid"),
                test_space("!space-c:example.invalid"),
            ],
            rooms: Vec::new(),
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state.navigation.space_order,
        vec![
            "!space-b:example.invalid",
            "!space-a:example.invalid",
            "!space-c:example.invalid"
        ]
    );
    assert_eq!(
        state
            .spaces
            .iter()
            .map(|space| space.space_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "!space-b:example.invalid",
            "!space-a:example.invalid",
            "!space-c:example.invalid"
        ]
    );
}

#[test]
fn loading_persisted_space_order_reorders_existing_spaces() {
    let mut state = ready_state();
    state.spaces = vec![
        test_space("!space-a:example.invalid"),
        test_space("!space-b:example.invalid"),
    ];

    let effects = reduce(
        &mut state,
        AppAction::NavigationLoaded {
            navigation: NavigationState {
                space_order: vec![
                    "!space-b:example.invalid".to_owned(),
                    "!space-a:example.invalid".to_owned(),
                ],
                ..NavigationState::default()
            },
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
    assert_eq!(
        state
            .spaces
            .iter()
            .map(|space| space.space_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!space-b:example.invalid", "!space-a:example.invalid"]
    );
}

#[test]
fn live_signal_actions_update_rust_owned_state() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::LiveRoomReceiptsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            receipts_by_event: vec![LiveEventReceipts {
                event_id: "$event:example.invalid".to_owned(),
                receipts: vec![LiveReadReceipt {
                    user_id: "@bob:example.invalid".to_owned(),
                    display_name: None,
                    original_display_label: String::new(),
                    avatar: None,
                    timestamp_ms: Some(1_234),
                }],
            }],
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    );
    assert_eq!(
        state.live_signals.rooms.get("!room:example.invalid"),
        Some(&RoomLiveSignals {
            receipts_by_event: [(
                "$event:example.invalid".to_owned(),
                LiveEventReceiptSummary {
                    readers: vec![LiveReadReceipt {
                        user_id: "@bob:example.invalid".to_owned(),
                        display_name: Some("Unknown user".to_owned()),
                        original_display_label: "Unknown user".to_owned(),
                        avatar: None,
                        timestamp_ms: Some(1_234),
                    }],
                    total_count: 1,
                    overflow_count: 0,
                },
            )]
            .into(),
            fully_read_event_id: None,
            typing_user_ids: Vec::new(),
            typing_users: Vec::new(),
        })
    );

    let effects = reduce(
        &mut state,
        AppAction::FullyReadMarkerUpdated {
            room_id: "!room:example.invalid".to_owned(),
            event_id: Some("$event:example.invalid".to_owned()),
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    );
    assert_eq!(
        state
            .live_signals
            .rooms
            .get("!room:example.invalid")
            .and_then(|room| room.fully_read_event_id.as_deref()),
        Some("$event:example.invalid")
    );

    let mut unread_room = test_room("!room:example.invalid", None);
    unread_room.unread_count = 3;
    unread_room.notification_count = 2;
    unread_room.highlight_count = 1;
    unread_room.marked_unread = true;
    state.rooms = vec![unread_room];

    let effects = reduce(
        &mut state,
        AppAction::FullyReadMarkerUpdated {
            room_id: "!room:example.invalid".to_owned(),
            event_id: Some("$event-2:example.invalid".to_owned()),
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    );
    let room = state
        .rooms
        .iter()
        .find(|room| room.room_id == "!room:example.invalid")
        .expect("room summary should exist");
    assert_eq!(room.unread_count, 3);
    assert_eq!(room.notification_count, 2);
    assert_eq!(room.highlight_count, 1);
    assert!(room.marked_unread);

    let effects = reduce(
        &mut state,
        AppAction::TypingUsersUpdated {
            room_id: "!room:example.invalid".to_owned(),
            user_ids: vec![
                "@carol:example.invalid".to_owned(),
                "@bob:example.invalid".to_owned(),
                "@bob:example.invalid".to_owned(),
            ],
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    );
    assert_eq!(
        state
            .live_signals
            .rooms
            .get("!room:example.invalid")
            .map(|room| room.typing_user_ids.clone()),
        Some(vec![
            "@bob:example.invalid".to_owned(),
            "@carol:example.invalid".to_owned(),
        ])
    );

    let effects = reduce(
        &mut state,
        AppAction::PresenceUpdated {
            user_id: "@bob:example.invalid".to_owned(),
            presence: PresenceKind::Away,
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    );
    assert_eq!(
        state.live_signals.presence.get("@bob:example.invalid"),
        Some(&PresenceKind::Away)
    );
}

#[test]
fn room_list_update_does_not_reintroduce_stale_unread_after_fully_read_marker() {
    let mut state = ready_state();
    let latest_event = latest_event("$latest:example.invalid", 42);
    let mut room = test_room("!room:example.invalid", None);
    room.latest_event = Some(latest_event.clone());
    room.recency_stamp = Some(42);
    state.rooms = vec![room];

    reduce(
        &mut state,
        AppAction::FullyReadMarkerUpdated {
            room_id: "!room:example.invalid".to_owned(),
            event_id: Some("$latest:example.invalid".to_owned()),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
        },
    );

    let mut stale_room = test_room("!room:example.invalid", None);
    stale_room.unread_count = 2;
    stale_room.notification_count = 2;
    stale_room.highlight_count = 1;
    stale_room.marked_unread = true;
    stale_room.latest_event = Some(latest_event);
    stale_room.recency_stamp = Some(42);
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![stale_room],
        },
    );

    let room = state
        .rooms
        .iter()
        .find(|room| room.room_id == "!room:example.invalid")
        .expect("room summary should exist");
    assert_eq!(room.unread_count, 0);
    assert_eq!(room.notification_count, 0);
    assert_eq!(room.highlight_count, 0);
    assert!(!room.marked_unread);
}

#[test]
fn room_list_update_suppresses_stale_unread_when_read_marker_event_differs_from_latest_event() {
    let mut state = ready_state();
    let latest_event = latest_event("$room-summary-latest:example.invalid", 42);
    let mut room = test_room("!room:example.invalid", None);
    room.latest_event = Some(latest_event.clone());
    room.recency_stamp = Some(42);
    state.rooms = vec![room];

    reduce(
        &mut state,
        AppAction::FullyReadMarkerUpdated {
            room_id: "!room:example.invalid".to_owned(),
            event_id: Some("$visible-read-event:example.invalid".to_owned()),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
        },
    );

    let mut stale_room = test_room("!room:example.invalid", None);
    stale_room.unread_count = 1;
    stale_room.notification_count = 1;
    stale_room.latest_event = Some(latest_event);
    stale_room.recency_stamp = Some(42);
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![stale_room],
        },
    );

    let room = state
        .rooms
        .iter()
        .find(|room| room.room_id == "!room:example.invalid")
        .expect("room summary should exist");
    assert_eq!(room.unread_count, 0);
    assert_eq!(room.notification_count, 0);
}

#[test]
fn room_list_update_preserves_unread_when_latest_event_changes_after_local_read() {
    let mut state = ready_state();
    let mut room = test_room("!room:example.invalid", None);
    room.latest_event = Some(latest_event("$old-latest:example.invalid", 42));
    room.recency_stamp = Some(42);
    state.rooms = vec![room];

    reduce(
        &mut state,
        AppAction::FullyReadMarkerUpdated {
            room_id: "!room:example.invalid".to_owned(),
            event_id: Some("$visible-read-event:example.invalid".to_owned()),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
        },
    );

    let mut new_unread_room = test_room("!room:example.invalid", None);
    new_unread_room.unread_count = 1;
    new_unread_room.notification_count = 1;
    new_unread_room.latest_event = Some(latest_event("$new-latest:example.invalid", 43));
    new_unread_room.recency_stamp = Some(43);
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![new_unread_room],
        },
    );

    let room = state
        .rooms
        .iter()
        .find(|room| room.room_id == "!room:example.invalid")
        .expect("room summary should exist");
    assert_eq!(room.unread_count, 1);
    assert_eq!(room.notification_count, 1);
}

#[test]
fn room_list_update_refreshes_native_attention_badge_without_initial_sync_sound_candidate() {
    let mut state = ready_state();
    let mut unread_room = test_room("!room:example.invalid", None);
    unread_room.unread_count = 3;
    unread_room.notification_count = 3;
    unread_room.highlight_count = 1;

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![unread_room],
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)));
    assert_eq!(state.native_attention.summary.unread_count, 3);
    assert_eq!(state.native_attention.summary.badge_count, 3);
    assert_eq!(state.native_attention.summary.highlight_count, 1);
    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(
        state.native_attention.dispatch,
        crate::state::NativeAttentionDispatchState::Suppressed {
            reason: crate::state::NativeAttentionSuppressionReason::InitialSync,
        }
    );
}

#[test]
fn room_list_update_creates_native_attention_candidate_for_live_unread_in_other_room() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("!active:example.invalid".to_owned());

    let active_room = test_room("!active:example.invalid", None);
    let quiet_room = test_room("!other:example.invalid", None);
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![active_room, quiet_room],
        },
    );

    let active_room = test_room("!active:example.invalid", None);
    let mut other_room = test_room("!other:example.invalid", None);
    other_room.unread_count = 1;
    other_room.notification_count = 1;
    other_room.latest_event = Some(latest_event("$new:example.invalid", 43));
    other_room.recency_stamp = Some(43);

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![active_room, other_room],
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)));
    assert_eq!(state.native_attention.summary.unread_count, 1);
    assert_eq!(state.native_attention.summary.badge_count, 1);
    assert_eq!(
        state.native_attention.summary.candidate,
        Some(crate::state::NativeAttentionCandidate {
            room_display_name: "!other:example.invalid".to_owned(),
            kind: crate::state::RoomAttentionKind::Message,
            unread_count: 1,
            highlight_count: 0,
        })
    );
    assert_eq!(
        state.native_attention.dispatch,
        crate::state::NativeAttentionDispatchState::Idle
    );
}

#[test]
fn room_list_update_suppresses_native_attention_candidate_for_live_unread_in_active_room() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("!active:example.invalid".to_owned());

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![test_room("!active:example.invalid", None)],
        },
    );

    let mut active_room = test_room("!active:example.invalid", None);
    active_room.unread_count = 2;
    active_room.notification_count = 2;
    active_room.latest_event = Some(latest_event("$active-new:example.invalid", 44));
    active_room.recency_stamp = Some(44);

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![active_room],
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)));
    assert_eq!(state.native_attention.summary.unread_count, 2);
    assert_eq!(state.native_attention.summary.badge_count, 2);
    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(
        state.native_attention.dispatch,
        crate::state::NativeAttentionDispatchState::Suppressed {
            reason: crate::state::NativeAttentionSuppressionReason::WindowFocused,
        }
    );
}

#[test]
fn unfocused_active_room_live_unread_creates_native_attention_candidate() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("!active:example.invalid".to_owned());

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![test_room("!active:example.invalid", None)],
        },
    );
    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: false,
            observation_generation: 1,
        },
    );

    let mut active_room = test_room("!active:example.invalid", None);
    active_room.unread_count = 2;
    active_room.notification_count = 2;
    active_room.latest_event = Some(latest_event("$active-new:example.invalid", 44));
    active_room.recency_stamp = Some(44);

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![active_room],
        },
    );

    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)));
    assert_eq!(state.native_attention.summary.badge_count, 2);
    assert_eq!(
        state.native_attention.summary.candidate,
        Some(crate::state::NativeAttentionCandidate {
            room_display_name: "!active:example.invalid".to_owned(),
            kind: crate::state::RoomAttentionKind::Message,
            unread_count: 2,
            highlight_count: 0,
        })
    );
    assert_eq!(
        state.native_attention.dispatch,
        crate::state::NativeAttentionDispatchState::Idle
    );
}

#[test]
fn focus_change_does_not_replay_existing_native_attention_candidate() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("!active:example.invalid".to_owned());

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![
                test_room("!active:example.invalid", None),
                test_room("!other:example.invalid", None),
            ],
        },
    );

    let mut other_room = test_room("!other:example.invalid", None);
    other_room.unread_count = 1;
    other_room.notification_count = 1;
    other_room.latest_event = Some(latest_event("$other-new:example.invalid", 45));
    other_room.recency_stamp = Some(45);
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![test_room("!active:example.invalid", None), other_room],
        },
    );
    assert!(state.native_attention.summary.candidate.is_some());

    let unread_count = state.native_attention.summary.unread_count;
    let badge_count = state.native_attention.summary.badge_count;
    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: false,
            observation_generation: 1,
        },
    );
    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: true,
            observation_generation: 2,
        },
    );

    assert_eq!(
        state.native_attention.summary.unread_count, unread_count,
        "focus changes must preserve unread totals"
    );
    assert_eq!(
        state.native_attention.summary.badge_count, badge_count,
        "focus changes must preserve badge totals"
    );
    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(
        state.native_attention.dispatch,
        crate::state::NativeAttentionDispatchState::Idle
    );
}

#[test]
fn native_window_focus_generation_rejects_stale_async_delivery() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: false,
            observation_generation: 1,
        },
    );
    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: true,
            observation_generation: 3,
        },
    );
    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: false,
            observation_generation: 2,
        },
    );

    assert!(state.native_attention_context.window_focused);
    assert_eq!(
        state
            .native_attention_context
            .window_focus_observation_generation,
        3
    );
}

#[test]
fn live_signals_clear_with_session_views() {
    let mut state = ready_state();
    state.live_signals.rooms.insert(
        "!room:example.invalid".to_owned(),
        RoomLiveSignals {
            fully_read_event_id: Some("$event:example.invalid".to_owned()),
            ..RoomLiveSignals::default()
        },
    );
    state
        .live_signals
        .presence
        .insert("@bob:example.invalid".to_owned(), PresenceKind::Online);

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert!(state.live_signals.rooms.is_empty());
    assert!(state.live_signals.presence.is_empty());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)))
    );
}

#[test]
fn live_read_receipts_project_reader_profiles_order_and_overflow() {
    let mut state = ready_state();
    state.profile.users.insert(
        "@alice:example.invalid".to_owned(),
        UserProfile {
            user_id: "@alice:example.invalid".to_owned(),
            display_name: Some("Alice".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: Some(AvatarImage {
                mxc_uri: "mxc://example.invalid/alice".to_owned(),
                thumbnail: AvatarThumbnailState::NotRequested,
            }),
        },
    );
    state.profile.users.insert(
        "@bob:example.invalid".to_owned(),
        UserProfile {
            user_id: "@bob:example.invalid".to_owned(),
            display_name: Some("Bob".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: Some(AvatarImage {
                mxc_uri: "mxc://example.invalid/bob".to_owned(),
                thumbnail: AvatarThumbnailState::NotRequested,
            }),
        },
    );
    state.profile.users.insert(
        "@carol:example.invalid".to_owned(),
        UserProfile {
            user_id: "@carol:example.invalid".to_owned(),
            display_name: Some("Carol".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::LiveRoomReceiptsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            receipts_by_event: vec![LiveEventReceipts {
                event_id: "$event:example.invalid".to_owned(),
                receipts: vec![
                    LiveReadReceipt {
                        user_id: "@alice:example.invalid".to_owned(),
                        display_name: None,
                        original_display_label: String::new(),
                        avatar: None,
                        timestamp_ms: Some(1_000),
                    },
                    LiveReadReceipt {
                        user_id: "@bob:example.invalid".to_owned(),
                        display_name: None,
                        original_display_label: String::new(),
                        avatar: None,
                        timestamp_ms: Some(3_000),
                    },
                    LiveReadReceipt {
                        user_id: "@carol:example.invalid".to_owned(),
                        display_name: None,
                        original_display_label: String::new(),
                        avatar: None,
                        timestamp_ms: Some(2_000),
                    },
                    LiveReadReceipt {
                        user_id: "@dana:example.invalid".to_owned(),
                        display_name: Some("Dana".to_owned()),
                        original_display_label: String::new(),
                        avatar: None,
                        timestamp_ms: Some(4_000),
                    },
                    LiveReadReceipt {
                        user_id: "@alice:example.invalid".to_owned(),
                        display_name: None,
                        original_display_label: String::new(),
                        avatar: None,
                        timestamp_ms: Some(5_000),
                    },
                ],
            }],
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    );
    let summary = state
        .live_signals
        .rooms
        .get("!room:example.invalid")
        .and_then(|room| room.receipts_by_event.get("$event:example.invalid"))
        .expect("receipt projection");
    // The session user (@alice) is excluded from the readers list — own
    // receipts must never appear in the displayed readers or affect counts.
    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.overflow_count, 0);
    assert_eq!(
        summary
            .readers
            .iter()
            .map(|receipt| (
                receipt.user_id.as_str(),
                receipt.display_name.as_deref(),
                receipt.timestamp_ms,
                receipt
                    .avatar
                    .as_ref()
                    .map(|avatar| avatar.mxc_uri.as_str()),
            ))
            .collect::<Vec<_>>(),
        vec![
            ("@dana:example.invalid", Some("Dana"), Some(4_000), None),
            (
                "@bob:example.invalid",
                Some("Bob"),
                Some(3_000),
                Some("mxc://example.invalid/bob"),
            ),
            ("@carol:example.invalid", Some("Carol"), Some(2_000), None),
        ]
    );
}

#[test]
fn media_download_updated_stores_state_for_active_room() {
    let mut state = ready_state();
    state.timeline.room_id = Some("!r:example.invalid".to_owned());

    let effects = reduce(
        &mut state,
        AppAction::MediaDownloadUpdated {
            room_id: "!r:example.invalid".to_owned(),
            event_id: "$ev:example.invalid".to_owned(),
            state: TimelineMediaDownloadState::Pending {
                progress: Some(MediaTransferProgress {
                    current: 3,
                    total: 10,
                }),
            },
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: "!r:example.invalid".to_owned(),
        })]
    );
    assert_eq!(state.timeline.media_downloads.len(), 1);
    let download = state
        .timeline
        .media_downloads
        .get("$ev:example.invalid")
        .expect("download entry");
    assert!(matches!(
        download,
        TimelineMediaDownloadState::Pending {
            progress: Some(MediaTransferProgress {
                current: 3,
                total: 10
            })
        }
    ));
}

#[test]
fn media_download_failed_does_not_replace_ready_media_for_active_room() {
    let mut state = ready_state();
    state.timeline.room_id = Some("!r:example.invalid".to_owned());
    state.timeline.media_downloads.insert(
        "$ev:example.invalid".to_owned(),
        TimelineMediaDownloadState::Ready {
            source_url: "/tmp/koushi-media.bin".to_owned(),
            width: Some(100),
            height: Some(80),
            mime_type: Some("image/png".to_owned()),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::MediaDownloadUpdated {
            room_id: "!r:example.invalid".to_owned(),
            event_id: "$ev:example.invalid".to_owned(),
            state: TimelineMediaDownloadState::Failed {
                failure_kind: OperationFailureKind::Sdk,
            },
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: "!r:example.invalid".to_owned(),
        })]
    );
    assert_eq!(
        state.timeline.media_downloads.get("$ev:example.invalid"),
        Some(&TimelineMediaDownloadState::Ready {
            source_url: "/tmp/koushi-media.bin".to_owned(),
            width: Some(100),
            height: Some(80),
            mime_type: Some("image/png".to_owned()),
        })
    );
}

#[test]
fn media_download_updated_ignored_for_inactive_room() {
    let mut state = ready_state();
    state.timeline.room_id = Some("!r:example.invalid".to_owned());

    let effects = reduce(
        &mut state,
        AppAction::MediaDownloadUpdated {
            room_id: "!other:example.invalid".to_owned(),
            event_id: "$ev:example.invalid".to_owned(),
            state: TimelineMediaDownloadState::Ready {
                source_url: "/tmp/x.png".to_owned(),
                width: Some(100),
                height: Some(100),
                mime_type: Some("image/png".to_owned()),
            },
        },
    );

    assert!(effects.is_empty());
    assert!(state.timeline.media_downloads.is_empty());
}

#[test]
fn media_download_updated_ignored_without_ready_session() {
    let mut state = AppState::default();
    state.timeline.room_id = Some("!r:example.invalid".to_owned());

    let effects = reduce(
        &mut state,
        AppAction::MediaDownloadUpdated {
            room_id: "!r:example.invalid".to_owned(),
            event_id: "$ev:example.invalid".to_owned(),
            state: TimelineMediaDownloadState::Failed {
                failure_kind: OperationFailureKind::Network,
            },
        },
    );

    assert!(effects.is_empty());
    assert!(state.timeline.media_downloads.is_empty());
}
