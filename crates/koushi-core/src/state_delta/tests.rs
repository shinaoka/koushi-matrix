use super::*;
use koushi_state::{
    AccountManagementUrl, ActivityRow, ActivityState, ActivityStream, DeviceCleanupOfferReason,
    DeviceCleanupState, InvitePreview, LiveEventReceiptSummary, MentionCandidatesCompleteness,
    MentionCandidatesTarget, MentionSurface, RoomInteractionState, RoomLiveSignals,
    RoomMentionPermission, RoomNotificationSettings, RoomSummary, RoomTags, SearchCrawlerRoomState,
    SecureBackupGateState, SpaceSummary, UserProfile,
};

#[test]
fn state_delta_contains_only_changed_slices_and_sidebar_projection() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.search_crawler.rooms.insert(
        "!room:example.invalid".to_owned(),
        SearchCrawlerRoomState::Queued,
    );

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert_eq!(delta.generation, 1);
    assert!(delta.changed.search_crawler.is_none());
    assert_eq!(
        delta.changed.search_crawler_rooms_by_id,
        Some(std::collections::BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(SearchCrawlerRoomState::Queued)
        )]))
    );
    assert!(delta.changed.session.is_none());
    assert!(delta.changed.sidebar.is_none());
}

#[test]
fn search_crawler_room_removal_is_an_explicit_scoped_delta() {
    let mut previous = AppState::default();
    previous.search_crawler.rooms.insert(
        "!room:example.invalid".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 3 },
    );

    let delta = build_state_delta(2, &previous, &AppState::default()).expect("state changed");

    assert!(delta.changed.search_crawler.is_none());
    assert_eq!(
        delta.changed.search_crawler_rooms_by_id,
        Some(std::collections::BTreeMap::from([(
            "!room:example.invalid".into(),
            None
        )]))
    );
}

#[test]
fn search_crawler_last_active_change_uses_a_scoped_slice() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.search_crawler.last_active = Some(koushi_state::SearchCrawlerLastActive {
        room_id: "!room:example.invalid".into(),
        updated_at_ms: 42,
        status: koushi_state::SearchCrawlerLastActiveStatus::Running,
        processed: 2,
        indexed: 1,
    });

    let delta = build_state_delta(3, &previous, &next).expect("state changed");

    assert_eq!(
        delta.changed.search_crawler_last_active,
        Some(next.search_crawler.last_active.clone())
    );
    assert!(delta.changed.search_crawler.is_none());
    assert!(delta.changed.search_crawler_rooms_by_id.is_none());
}

#[test]
fn activity_row_updates_use_scoped_deltas_when_stream_shape_is_stable() {
    let mut previous = AppState::default();
    previous.activity = ActivityState::Open {
        active_tab: Default::default(),
        recent: ActivityStream {
            rows: vec![ActivityRow::event(
                "!room:example.invalid".into(),
                "$event:example.invalid".into(),
                None,
                "Room".into(),
                None,
                Some("old".into()),
                1,
                true,
                false,
            )],
            ..Default::default()
        },
        unread: ActivityStream::default(),
        mark_read: Default::default(),
    };
    let mut next = previous.clone();
    if let ActivityState::Open { recent, .. } = &mut next.activity {
        recent.rows[0].preview = Some("new".into());
    }

    let delta = build_state_delta(4, &previous, &next).expect("state changed");

    assert!(delta.changed.activity.is_none());
    assert_eq!(
        delta.changed.activity_recent_rows_by_id,
        Some(BTreeMap::from([(
            "event:$event:example.invalid".into(),
            Some(match &next.activity {
                ActivityState::Open { recent, .. } => recent.rows[0].clone(),
                _ => unreachable!(),
            })
        )]))
    );
    assert!(delta.changed.activity_unread_rows_by_id.is_none());
}

#[test]
fn activity_order_or_metadata_changes_keep_the_full_slice() {
    let mut previous = AppState::default();
    previous.activity = ActivityState::Open {
        active_tab: Default::default(),
        recent: ActivityStream {
            rows: vec![ActivityRow::event(
                "!room:example.invalid".into(),
                "$event:example.invalid".into(),
                None,
                "Room".into(),
                None,
                None,
                1,
                true,
                false,
            )],
            ..Default::default()
        },
        unread: ActivityStream::default(),
        mark_read: Default::default(),
    };
    let mut next = previous.clone();
    if let ActivityState::Open { recent, .. } = &mut next.activity {
        recent.next_batch = Some("next".into());
    }

    let delta = build_state_delta(5, &previous, &next).expect("state changed");

    assert_eq!(delta.changed.activity, Some(next.activity));
    assert!(delta.changed.activity_recent_rows_by_id.is_none());
}

#[test]
fn account_management_url_clear_is_an_explicit_delta() {
    let mut previous = AppState::default();
    previous.account_management_url = Some(AccountManagementUrl::from_validated(
        "https://account.example/devices".to_owned(),
    ));
    let next = AppState::default();

    let delta = build_state_delta(2, &previous, &next).expect("URL clear changed state");

    assert_eq!(delta.changed.account_management_url, Some(None));
}

#[test]
fn state_delta_omits_unchanged_state() {
    assert!(build_state_delta(1, &AppState::default(), &AppState::default()).is_none());
}

#[test]
fn session_lock_reason_delta_preserves_nested_some_and_explicit_none() {
    let mut locked = AppState::default();
    locked.session_lock_reason =
        Some(koushi_state::SessionLockReason::UnknownToken { soft_logout: true });
    let delta = build_state_delta(2, &AppState::default(), &locked).expect("reason changed");
    assert_eq!(
        delta.changed.session_lock_reason,
        Some(Some(koushi_state::SessionLockReason::UnknownToken {
            soft_logout: true,
        }))
    );

    let clear = build_state_delta(3, &locked, &AppState::default()).expect("reason cleared");
    assert_eq!(clear.changed.session_lock_reason, Some(None));
}

#[test]
fn device_cleanup_state_is_an_incremental_slice() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.device_cleanup = DeviceCleanupState::Offered {
        reason: DeviceCleanupOfferReason::RecoveryFailed,
    };

    let delta = build_state_delta(7, &previous, &next).expect("cleanup state changed");

    assert_eq!(delta.changed.device_cleanup, Some(next.device_cleanup));
    let mut without_cleanup = delta.changed;
    without_cleanup.device_cleanup = None;
    assert!(without_cleanup.is_empty());
}

#[test]
fn secure_backup_gate_is_an_incremental_slice() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.secure_backup_gate = SecureBackupGateState::Checking;

    let delta = build_state_delta(8, &previous, &next).expect("backup gate changed");

    assert_eq!(
        delta.changed.secure_backup_gate,
        Some(SecureBackupGateState::Checking)
    );
    let mut without_gate = delta.changed;
    without_gate.secure_backup_gate = None;
    assert!(without_gate.is_empty());
}

#[test]
fn state_delta_emits_only_the_changed_mention_candidates_slice() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.mention_candidates
        .targets
        .push(MentionCandidatesTarget {
            room_id: "!room:example.invalid".to_owned(),
            generation: 1,
            request_id: 2,
            query: "ali".to_owned(),
            surface: MentionSurface::Main,
            completeness: MentionCandidatesCompleteness::Partial,
            candidates: Vec::new(),
            room_mention_allowed: RoomMentionPermission::Allowed,
            failure_kind: None,
        });

    let delta = build_state_delta(2, &previous, &next).expect("mention candidates changed");

    assert_eq!(
        delta.changed.mention_candidates,
        Some(next.mention_candidates)
    );
    let mut without_mentions = delta.changed;
    without_mentions.mention_candidates = None;
    assert!(without_mentions.is_empty());
}

#[test]
fn navigation_delta_retains_event_navigation() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    next.navigation.event_navigation = koushi_state::EventNavigationState::Opening {
        generation: 1,
        source: koushi_state::EventNavigationSource::Activity,
    };

    let delta = build_state_delta(1, &previous, &next).expect("navigation changed");

    assert_eq!(delta.changed.navigation, Some(next.navigation.clone()));
    assert_eq!(
        delta
            .changed
            .navigation
            .as_ref()
            .map(|navigation| navigation.event_navigation),
        Some(koushi_state::EventNavigationState::Opening {
            generation: 1,
            source: koushi_state::EventNavigationSource::Activity,
        })
    );
    assert!(delta.changed.sidebar.is_none());
}

#[test]
fn live_signal_room_changes_use_a_scoped_delta() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.live_signals
        .rooms
        .insert("!room:example.invalid".into(), RoomLiveSignals::default());

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.live_signals.is_none());
    assert_eq!(
        delta.changed.live_signals_rooms,
        Some(std::collections::BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(RoomLiveSignals::default())
        )]))
    );
}

fn room(id: &str) -> RoomSummary {
    RoomSummary {
        room_id: id.into(),
        display_name: id.into(),
        display_label: id.into(),
        original_display_label: String::new(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
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
fn live_signal_receipt_event_changes_use_nested_scoped_deltas() {
    let mut previous = AppState::default();
    let room = previous
        .live_signals
        .rooms
        .entry("!room:example.invalid".into())
        .or_default();
    room.receipts_by_event.insert(
        "$changed:example.invalid".into(),
        LiveEventReceiptSummary {
            total_count: 1,
            ..Default::default()
        },
    );
    room.receipts_by_event.insert(
        "$removed:example.invalid".into(),
        LiveEventReceiptSummary {
            total_count: 2,
            ..Default::default()
        },
    );
    let mut next = previous.clone();
    let room = next
        .live_signals
        .rooms
        .get_mut("!room:example.invalid")
        .unwrap();
    room.receipts_by_event
        .get_mut("$changed:example.invalid")
        .unwrap()
        .total_count = 3;
    room.receipts_by_event.remove("$removed:example.invalid");
    room.receipts_by_event.insert(
        "$added:example.invalid".into(),
        LiveEventReceiptSummary {
            total_count: 4,
            ..Default::default()
        },
    );

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.live_signals.is_none());
    assert!(delta.changed.live_signals_rooms.is_none());
    assert_eq!(
        delta.changed.live_signals_receipts_by_room_event,
        Some(BTreeMap::from([(
            "!room:example.invalid".into(),
            BTreeMap::from([
                (
                    "$added:example.invalid".into(),
                    Some(LiveEventReceiptSummary {
                        total_count: 4,
                        ..Default::default()
                    }),
                ),
                (
                    "$changed:example.invalid".into(),
                    Some(LiveEventReceiptSummary {
                        total_count: 3,
                        ..Default::default()
                    }),
                ),
                ("$removed:example.invalid".into(), None),
            ]),
        )]))
    );
}

#[test]
fn live_signal_room_metadata_changes_use_a_scoped_delta() {
    let mut previous = AppState::default();
    previous
        .live_signals
        .rooms
        .insert("!room:example.invalid".into(), RoomLiveSignals::default());
    let mut next = previous.clone();
    next.live_signals
        .rooms
        .get_mut("!room:example.invalid")
        .unwrap()
        .fully_read_event_id = Some("$read:example.invalid".into());

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.live_signals.is_none());
    assert!(delta.changed.live_signals_rooms.is_none());
    assert!(delta.changed.live_signals_receipts_by_room_event.is_none());
    assert_eq!(
        delta.changed.live_signals_room_metadata_by_id,
        Some(BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(koushi_protocol::RoomLiveSignalMetadata {
                fully_read_event_id: Some("$read:example.invalid".into()),
                typing_user_ids: Vec::new(),
                typing_users: Vec::new(),
            }),
        )]))
    );
}

#[test]
fn live_signal_receipt_and_metadata_changes_remain_separate_scoped_deltas() {
    let mut previous = AppState::default();
    let room = previous
        .live_signals
        .rooms
        .entry("!room:example.invalid".into())
        .or_default();
    room.receipts_by_event.insert(
        "$event:example.invalid".into(),
        LiveEventReceiptSummary {
            total_count: 1,
            ..Default::default()
        },
    );

    let mut next = previous.clone();
    let room = next
        .live_signals
        .rooms
        .get_mut("!room:example.invalid")
        .unwrap();
    room.fully_read_event_id = Some("$read:example.invalid".into());
    room.receipts_by_event
        .get_mut("$event:example.invalid")
        .unwrap()
        .total_count = 2;

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.live_signals.is_none());
    assert!(delta.changed.live_signals_rooms.is_none());
    assert_eq!(
        delta.changed.live_signals_room_metadata_by_id,
        Some(BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(koushi_protocol::RoomLiveSignalMetadata {
                fully_read_event_id: Some("$read:example.invalid".into()),
                typing_user_ids: Vec::new(),
                typing_users: Vec::new(),
            }),
        )]))
    );
    assert_eq!(
        delta.changed.live_signals_receipts_by_room_event,
        Some(BTreeMap::from([(
            "!room:example.invalid".into(),
            BTreeMap::from([(
                "$event:example.invalid".into(),
                Some(LiveEventReceiptSummary {
                    total_count: 2,
                    ..Default::default()
                }),
            )]),
        )]))
    );
}

#[test]
#[ignore = "explicit 100-room publication measurement"]
fn reference_live_signal_receipt_delta_measurement() {
    use std::time::Instant;

    let setup_started = Instant::now();
    let mut previous = AppState::default();
    for room_index in 0..100 {
        let room_id = format!("!room-{room_index}:example.invalid");
        let event_count = if room_index == 0 { 10_000 } else { 10 };
        let room = previous.live_signals.rooms.entry(room_id).or_default();
        for event_index in 0..event_count {
            let reader_count = if room_index == 0 && event_index == 0 {
                1_500
            } else {
                1
            };
            room.receipts_by_event.insert(
                format!("$event-{room_index}-{event_index}:example.invalid"),
                LiveEventReceiptSummary {
                    readers: Vec::new(),
                    total_count: reader_count,
                    overflow_count: reader_count.saturating_sub(3),
                },
            );
        }
    }
    let setup_ms = setup_started.elapsed().as_secs_f64() * 1_000.0;
    let event_count: usize = previous
        .live_signals
        .rooms
        .values()
        .map(|room| room.receipts_by_event.len())
        .sum();
    assert_eq!(previous.live_signals.rooms.len(), 100);
    assert_eq!(event_count, 10_990);

    let mut next = previous.clone();
    let active_room = next
        .live_signals
        .rooms
        .get_mut("!room-0:example.invalid")
        .unwrap();
    active_room
        .receipts_by_event
        .get_mut("$event-0-0:example.invalid")
        .unwrap()
        .total_count -= 1;
    active_room
        .receipts_by_event
        .get_mut("$event-0-1:example.invalid")
        .unwrap()
        .total_count += 1;

    let mut build_samples = Vec::with_capacity(25);
    let mut first_delta = None;
    for _ in 0..25 {
        let build_started = Instant::now();
        let candidate = build_state_delta(1, &previous, &next).expect("receipt move changes state");
        build_samples.push(build_started.elapsed().as_secs_f64() * 1_000.0);
        first_delta.get_or_insert(candidate);
    }
    build_samples.sort_by(f64::total_cmp);
    let delta = first_delta.expect("reference measurement has a delta");
    let build_ms = build_samples[0];
    let p95_ms = build_samples[(build_samples.len() * 95).div_ceil(100) - 1];
    let encoded_bytes = serde_json::to_vec(&delta)
        .expect("receipt delta serializes")
        .len();
    let touched_events = delta
        .changed
        .live_signals_receipts_by_room_event
        .as_ref()
        .and_then(|rooms| rooms.get("!room-0:example.invalid"))
        .map_or(0, BTreeMap::len);

    eprintln!(
        "scoped_publication_reference rooms=100 events={event_count} active_readers=1500 setup_ms={setup_ms:.2} build_ms={build_ms:.2} build_p95_ms={p95_ms:.2} encoded_delta_bytes={encoded_bytes} touched_events={touched_events}"
    );
    assert!(delta.changed.live_signals_rooms.is_none());
    assert_eq!(touched_events, 2);
    assert!(encoded_bytes < 4 * 1024);
}

#[test]
fn live_signal_presence_changes_use_a_scoped_delta() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.live_signals.presence.insert(
        "@reader:example.invalid".into(),
        koushi_state::PresenceKind::Online,
    );

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.live_signals.is_none());
    assert_eq!(
        delta.changed.live_signals_presence_by_user,
        Some(BTreeMap::from([(
            "@reader:example.invalid".into(),
            Some(koushi_state::PresenceKind::Online)
        )]))
    );
    assert!(delta.changed.live_signals_rooms.is_none());
}

#[test]
fn live_signal_presence_removal_is_an_explicit_scoped_delta() {
    let mut previous = AppState::default();
    previous.live_signals.presence.insert(
        "@reader:example.invalid".into(),
        koushi_state::PresenceKind::Away,
    );

    let delta = build_state_delta(2, &previous, &AppState::default()).expect("state changed");

    assert!(delta.changed.live_signals.is_none());
    assert_eq!(
        delta.changed.live_signals_presence_by_user,
        Some(BTreeMap::from([("@reader:example.invalid".into(), None)]))
    );
}

#[test]
fn room_changes_use_a_scoped_delta_when_order_is_stable() {
    let mut previous = AppState::default();
    previous.rooms.push(room("!room:example.invalid"));
    let mut next = previous.clone();
    next.rooms[0].unread_count = 1;

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.rooms.is_none());
    assert_eq!(
        delta.changed.rooms_by_id,
        Some(std::collections::BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(next.rooms[0].clone())
        )]))
    );
}

#[test]
fn room_removal_uses_a_scoped_delta_when_surviving_order_is_stable() {
    let mut previous = AppState::default();
    previous.rooms.push(room("!room-removed:example.invalid"));
    previous.rooms.push(room("!room-surviving:example.invalid"));
    let mut next = previous.clone();
    next.rooms.remove(0);

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.rooms.is_none());
    assert_eq!(
        delta.changed.rooms_by_id,
        Some(BTreeMap::from([(
            "!room-removed:example.invalid".into(),
            None
        )]))
    );
}

#[test]
fn hundred_room_update_publishes_one_room_replacement() {
    let mut previous = AppState::default();
    previous.rooms = (0..100)
        .map(|index| room(&format!("!room-{index}:example.invalid")))
        .collect();
    let mut next = previous.clone();
    next.rooms[42].unread_count = 1;

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.rooms.is_none());
    let changes = delta.changed.rooms_by_id.expect("scoped room changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes["!room-42:example.invalid"],
        Some(next.rooms[42].clone())
    );
}

#[test]
fn space_changes_use_a_scoped_delta_when_order_is_stable() {
    let mut previous = AppState::default();
    previous.spaces.push(SpaceSummary {
        space_id: "!space:example.invalid".into(),
        display_name: "Space".into(),
        avatar: None,
        child_room_ids: Vec::new(),
    });
    let mut next = previous.clone();
    next.spaces[0].display_name = "Renamed Space".into();

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.spaces.is_none());
    assert_eq!(
        delta.changed.spaces_by_id,
        Some(std::collections::BTreeMap::from([(
            "!space:example.invalid".into(),
            Some(next.spaces[0].clone())
        )]))
    );
}

#[test]
fn invite_changes_use_a_scoped_delta_when_order_is_stable() {
    let mut previous = AppState::default();
    previous.invites.push(InvitePreview {
        room_id: "!invite:example.invalid".into(),
        display_name: "Invite".into(),
        avatar: None,
        topic: None,
        inviter_display_name: Some("Inviter".into()),
        inviter_user_id: Some("@inviter:example.invalid".into()),
        is_dm: false,
    });
    let mut next = previous.clone();
    next.invites[0].topic = Some("Updated topic".into());

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.invites.is_none());
    assert_eq!(
        delta.changed.invites_by_id,
        Some(std::collections::BTreeMap::from([(
            "!invite:example.invalid".into(),
            Some(next.invites[0].clone())
        )]))
    );
}

#[test]
fn global_profile_user_changes_use_a_scoped_delta() {
    let mut previous = AppState::default();
    previous.profile.users.insert(
        "@user:example.invalid".into(),
        UserProfile {
            user_id: "@user:example.invalid".into(),
            display_name: Some("User".into()),
            display_label: "User".into(),
            original_display_label: "User".into(),
            mention_search_terms: vec!["user".into()],
            avatar: None,
        },
    );
    let mut next = previous.clone();
    next.profile
        .users
        .get_mut("@user:example.invalid")
        .unwrap()
        .display_label = "Renamed User".into();

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.profile.is_none());
    assert_eq!(
        delta.changed.profile_users_by_id,
        Some(std::collections::BTreeMap::from([(
            "@user:example.invalid".into(),
            Some(next.profile.users["@user:example.invalid"].clone())
        )]))
    );
}

#[test]
fn room_profile_user_changes_use_a_nested_scoped_delta() {
    let mut previous = AppState::default();
    previous.profile.room_users.insert(
        "!room:example.invalid".into(),
        [(
            "@user:example.invalid".into(),
            UserProfile {
                user_id: "@user:example.invalid".into(),
                display_name: Some("User".into()),
                display_label: "User".into(),
                original_display_label: "User".into(),
                mention_search_terms: vec!["user".into()],
                avatar: None,
            },
        )]
        .into(),
    );
    let mut next = previous.clone();
    next.profile
        .room_users
        .get_mut("!room:example.invalid")
        .unwrap()
        .get_mut("@user:example.invalid")
        .unwrap()
        .display_label = "Renamed User".into();

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.profile.is_none());
    assert_eq!(
        delta.changed.profile_room_users_by_room,
        Some(std::collections::BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(std::collections::BTreeMap::from([(
                "@user:example.invalid".into(),
                Some(
                    next.profile.room_users["!room:example.invalid"]["@user:example.invalid"]
                        .clone()
                )
            )]))
        )]))
    );
}

#[test]
fn simultaneous_global_and_room_profile_changes_remain_scoped() {
    let user = UserProfile {
        user_id: "@user:example.invalid".into(),
        display_name: Some("User".into()),
        display_label: "User".into(),
        original_display_label: "User".into(),
        mention_search_terms: vec!["user".into()],
        avatar: None,
    };
    let mut previous = AppState::default();
    previous
        .profile
        .users
        .insert(user.user_id.clone(), user.clone());
    previous.profile.room_users.insert(
        "!room:example.invalid".into(),
        [(user.user_id.clone(), user)].into(),
    );
    let mut next = previous.clone();
    next.profile
        .users
        .get_mut("@user:example.invalid")
        .unwrap()
        .display_label = "Global User".into();
    next.profile
        .room_users
        .get_mut("!room:example.invalid")
        .unwrap()
        .get_mut("@user:example.invalid")
        .unwrap()
        .display_label = "Room User".into();

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.profile.is_none());
    assert!(delta.changed.profile_users_by_id.is_some());
    assert!(delta.changed.profile_room_users_by_room.is_some());
}

#[test]
fn profile_scalar_and_local_collection_changes_remain_scoped() {
    let previous = AppState::default();
    let mut next = previous.clone();
    next.profile.own.display_name = Some("Own User".into());
    next.profile
        .local_aliases
        .insert("@user:example.invalid".into(), "Alias".into());
    next.profile
        .ignored_user_ids
        .insert("@ignored:example.invalid".into());
    next.profile.local_alias_update =
        koushi_state::LocalUserAliasUpdateState::Saving { request_id: 7 };
    next.profile.ignored_user_update =
        koushi_state::IgnoredUserUpdateState::Saving { request_id: 8 };
    next.profile.update = koushi_state::ProfileUpdateState::SettingDisplayName {
        request_id: 9,
        display_name: Some("Own User".into()),
    };

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.profile.is_none());
    assert_eq!(delta.changed.profile_own, Some(next.profile.own.clone()));
    assert_eq!(
        delta.changed.profile_local_aliases_by_id,
        Some(BTreeMap::from([(
            "@user:example.invalid".into(),
            Some("Alias".into())
        )]))
    );
    assert_eq!(
        delta.changed.profile_ignored_user_ids_by_id,
        Some(BTreeMap::from([("@ignored:example.invalid".into(), true)]))
    );
    assert_eq!(
        delta.changed.profile_local_alias_update,
        Some(next.profile.local_alias_update.clone())
    );
    assert_eq!(
        delta.changed.profile_ignored_user_update,
        Some(next.profile.ignored_user_update.clone())
    );
    assert_eq!(
        delta.changed.profile_update,
        Some(next.profile.update.clone())
    );
}

#[test]
fn room_policy_maps_use_scoped_deltas() {
    let mut previous = AppState::default();
    previous.room_notification_settings.insert(
        "!room:example.invalid".into(),
        RoomNotificationSettings::default(),
    );
    previous.room_interactions.insert(
        "!room:example.invalid".into(),
        RoomInteractionState::default(),
    );
    let mut next = previous.clone();
    next.room_notification_settings
        .get_mut("!room:example.invalid")
        .unwrap()
        .mode = koushi_state::RoomNotificationMode::Mentions;
    next.room_interactions
        .get_mut("!room:example.invalid")
        .unwrap()
        .pinned_events
        .push(koushi_state::PinnedEvent {
            event_id: "$event:example.invalid".into(),
            sender: None,
            sender_label: None,
            body_preview: None,
            redacted: false,
            timestamp_ms: None,
            state: Default::default(),
            thread_root_event_id: None,
        });

    let delta = build_state_delta(1, &previous, &next).expect("state changed");

    assert!(delta.changed.room_notification_settings.is_none());
    assert!(delta.changed.room_interactions.is_none());
    assert_eq!(
        delta.changed.room_notification_settings_by_id,
        Some(BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(next.room_notification_settings["!room:example.invalid"].clone())
        )]))
    );
    assert_eq!(
        delta.changed.room_interactions_by_id,
        Some(BTreeMap::from([(
            "!room:example.invalid".into(),
            Some(next.room_interactions["!room:example.invalid"].clone())
        )]))
    );
}
