use koushi_state::{
    AppAction, AppState, DisplayPlatform, NativeAttentionCandidate, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionDispatchId, NativeAttentionDispatchState,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, NativeAttentionSoundOutcome,
    NativeAttentionSuppressionReason, NotificationSettings, RoomAttentionKind, RoomSummary,
    RoomTagInfo, RoomTags, SessionInfo, SessionState, SettingsPatch,
    native_attention_capabilities_for_platform, native_attention_state_from_rooms, reduce,
    room_attention_summary,
};
use serde_json::json;

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.invalid".to_owned(),
            user_id: "@attention:example.invalid".to_owned(),
            device_id: "ATTENTION".to_owned(),
        }),
        ..AppState::default()
    }
}

fn room(
    room_id: &str,
    display_name: &str,
    is_dm: bool,
    unread_count: u64,
    notification_count: u64,
    highlight_count: u64,
) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        display_label: display_name.to_owned(),
        original_display_label: display_name.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count,
        notification_count,
        highlight_count,
        marked_unread: false,
        last_activity_ms: 0,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn available_capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

#[test]
fn native_attention_capabilities_are_resolved_from_platform_profile() {
    let macos = native_attention_capabilities_for_platform(DisplayPlatform::Macos);
    let windows = native_attention_capabilities_for_platform(DisplayPlatform::Windows);
    let linux = native_attention_capabilities_for_platform(DisplayPlatform::Linux);

    assert_eq!(macos.notifications, NativeAttentionCapability::Available);
    assert_eq!(windows.notifications, NativeAttentionCapability::Available);
    assert_eq!(linux.notifications, NativeAttentionCapability::Available);
    assert_eq!(macos.badge, NativeAttentionCapability::Available);
    assert_eq!(windows.badge, NativeAttentionCapability::Available);
    assert_eq!(linux.badge, NativeAttentionCapability::Unknown);
    assert_eq!(macos.overlay_icon, NativeAttentionCapability::Unavailable);
    assert_eq!(windows.overlay_icon, NativeAttentionCapability::Available);
    assert_eq!(linux.overlay_icon, NativeAttentionCapability::Unavailable);
    assert_eq!(macos.sound, NativeAttentionCapability::Available);
    assert_eq!(windows.sound, NativeAttentionCapability::Available);
    assert_eq!(linux.sound, NativeAttentionCapability::Unavailable);
    assert_eq!(macos.tray, NativeAttentionCapability::Unknown);
    assert_eq!(windows.tray, NativeAttentionCapability::Unknown);
    assert_eq!(linux.tray, NativeAttentionCapability::Unknown);
    assert_eq!(macos.activation, NativeAttentionCapability::Unknown);
    assert_eq!(windows.activation, NativeAttentionCapability::Unknown);
    assert_eq!(linux.activation, NativeAttentionCapability::Unknown);
}

#[test]
fn room_attention_summary_serializes_only_allowed_fields() {
    let summary = room_attention_summary("Room A".to_owned(), false, 7, 2, 7).unwrap();

    assert_eq!(
        serde_json::to_value(summary).unwrap(),
        json!({
            "room_display_name": "Room A",
            "kind": "mention",
            "notification_count": 7,
            "highlight_count": 2,
            "unread_count": 7,
        })
    );
}

#[test]
fn room_attention_summary_omits_payload_when_unread_is_absent() {
    assert_eq!(
        room_attention_summary("Room A".to_owned(), false, 0, 0, 0),
        None
    );
}

#[test]
fn room_attention_kind_prefers_mentions_over_dm_and_message() {
    assert_eq!(
        koushi_state::room_attention_kind(true, 4, 2, 4),
        Some(RoomAttentionKind::Mention)
    );
    assert_eq!(
        koushi_state::room_attention_kind(true, 4, 0, 4),
        Some(RoomAttentionKind::Dm)
    );
    assert_eq!(
        koushi_state::room_attention_kind(false, 4, 0, 4),
        Some(RoomAttentionKind::Message)
    );
}

#[test]
fn native_attention_candidate_prioritizes_mentions_dm_then_messages_and_badges() {
    let rooms = vec![
        room("!message:example.invalid", "Room", false, 8, 8, 0),
        room("!dm:example.invalid", "Direct", true, 3, 3, 0),
        room("!mention:example.invalid", "Mention", false, 1, 1, 1),
    ];

    let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    assert_eq!(state.summary.unread_count, 12);
    assert_eq!(state.summary.highlight_count, 1);
    assert_eq!(state.summary.badge_count, 12);
    assert_eq!(
        state.summary.candidate,
        Some(NativeAttentionCandidate {
            room_display_name: "Mention".to_owned(),
            kind: RoomAttentionKind::Mention,
            unread_count: 1,
            highlight_count: 1,
        })
    );
    assert_eq!(state.dispatch, NativeAttentionDispatchState::Idle);
}

#[test]
fn native_attention_ignores_plain_unread_counts_absent_from_activity_unread() {
    let rooms = vec![
        room("!plain:example.invalid", "Plain", false, 1, 0, 0),
        room("!notified:example.invalid", "Notified", false, 4, 2, 0),
    ];

    let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    assert_eq!(state.summary.unread_count, 2);
    assert_eq!(state.summary.badge_count, 2);
    assert_eq!(
        state.summary.candidate,
        Some(NativeAttentionCandidate {
            room_display_name: "Notified".to_owned(),
            kind: RoomAttentionKind::Message,
            unread_count: 2,
            highlight_count: 0,
        })
    );
}

#[test]
fn native_attention_candidate_uses_projected_room_display_label() {
    let mut dm = room("!dm:example.invalid", "Alice Upstream", true, 3, 3, 0);
    dm.display_label = "Alice Local".to_owned();

    let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &[dm],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    assert_eq!(
        state.summary.candidate,
        Some(NativeAttentionCandidate {
            room_display_name: "Alice Local".to_owned(),
            kind: RoomAttentionKind::Dm,
            unread_count: 3,
            highlight_count: 0,
        })
    );
}

#[test]
fn native_attention_candidate_serialization_omits_room_identity() {
    let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &[room("!dm:example.invalid", "Alice", true, 3, 3, 0)],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    let value = serde_json::to_value(state).expect("serialize native attention");
    let candidate = value["summary"]["candidate"]
        .as_object()
        .expect("candidate object");

    assert!(!candidate.contains_key("room_id"));
}

#[test]
fn native_attention_suppresses_initial_backfill_self_and_focused_room() {
    let rooms = vec![room("!room:example.invalid", "Room", false, 2, 2, 0)];

    for (observation, reason) in [
        (
            NativeAttentionObservationKind::InitialSync,
            NativeAttentionSuppressionReason::InitialSync,
        ),
        (
            NativeAttentionObservationKind::Backfill,
            NativeAttentionSuppressionReason::Backfill,
        ),
        (
            NativeAttentionObservationKind::SelfEvent,
            NativeAttentionSuppressionReason::SelfMessage,
        ),
    ] {
        let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
            rooms: &rooms,
            active_room_id: None,
            muted_room_ids: &[],
            room_notification_modes: &std::collections::HashMap::new(),
            ignored_user_ids: &std::collections::BTreeSet::new(),
            window_focused: false,
            observation,
            previous_candidate: None,
            capabilities: available_capabilities(),
        });

        assert_eq!(
            state.dispatch,
            NativeAttentionDispatchState::Suppressed { reason }
        );
        assert_eq!(state.summary.candidate, None);
    }

    let focused = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: Some("!room:example.invalid"),
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: true,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    assert_eq!(
        focused.dispatch,
        NativeAttentionDispatchState::Suppressed {
            reason: NativeAttentionSuppressionReason::WindowFocused
        }
    );
    assert_eq!(focused.summary.candidate, None);
}

#[test]
fn native_attention_excludes_low_priority_and_muted_rooms_and_clears_badge_at_zero() {
    let mut low_priority = room("!low:example.invalid", "Low", false, 5, 5, 1);
    low_priority.tags.low_priority = Some(RoomTagInfo {
        order: Some("0.9".to_owned()),
    });
    let muted = room("!muted:example.invalid", "Muted", false, 4, 4, 0);

    let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &[low_priority, muted],
        active_room_id: None,
        muted_room_ids: &["!muted:example.invalid".to_owned()],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    assert_eq!(state.summary.unread_count, 0);
    assert_eq!(state.summary.highlight_count, 0);
    assert_eq!(state.summary.badge_count, 0);
    assert_eq!(state.summary.candidate, None);
    assert_eq!(state.dispatch, NativeAttentionDispatchState::Idle);
}

#[test]
fn native_attention_capability_unavailable_and_duplicate_candidates_are_suppressed() {
    let rooms = vec![room("!room:example.invalid", "Room", false, 2, 2, 0)];
    let unavailable = NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Unavailable,
        ..available_capabilities()
    };

    let state = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: unavailable,
    });

    assert_eq!(state.summary.badge_count, 2);
    assert_eq!(state.summary.candidate, None);
    assert_eq!(
        state.dispatch,
        NativeAttentionDispatchState::Suppressed {
            reason: NativeAttentionSuppressionReason::CapabilityUnavailable
        }
    );

    let previous = NativeAttentionCandidate {
        room_display_name: "Room".to_owned(),
        kind: RoomAttentionKind::Message,
        unread_count: 2,
        highlight_count: 0,
    };
    let duplicate = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: Some(&previous),
        capabilities: available_capabilities(),
    });

    assert_eq!(duplicate.summary.candidate, None);
    assert_eq!(
        duplicate.dispatch,
        NativeAttentionDispatchState::Suppressed {
            reason: NativeAttentionSuppressionReason::Duplicate
        }
    );
}

#[test]
fn native_attention_reducer_preserves_dispatch_state_not_only_summary() {
    let rooms = vec![room("!room:example.invalid", "Room", false, 2, 2, 0)];
    let attention = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: Some("!room:example.invalid"),
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: true,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: available_capabilities(),
    });

    let mut app_state = ready_state();
    let effects = reduce(
        &mut app_state,
        AppAction::NativeAttentionUpdated {
            attention: attention.clone(),
        },
    );

    assert_eq!(app_state.native_attention.summary, attention.summary);
    assert_eq!(
        app_state.native_attention.dispatch,
        NativeAttentionDispatchState::Idle
    );
    assert_eq!(effects.len(), 1);
}

#[test]
fn disabling_badges_immediately_projects_zero_from_rust_owned_settings() {
    let mut state = AppState::default();
    state.native_attention.summary.unread_count = 7;
    state.native_attention.summary.badge_count = 7;
    state.native_attention.summary.capabilities.badge = NativeAttentionCapability::Available;
    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 44,
            patch: SettingsPatch {
                notifications: Some(NotificationSettings {
                    badges: false,
                    ..NotificationSettings::default()
                }),
                ..SettingsPatch::default()
            },
        },
    );
    assert_eq!(state.native_attention.summary.badge_count, 0);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(koushi_state::UiEvent::NativeAttentionChanged)
    )));
}

#[test]
fn native_sound_dispatch_outcomes_are_correlated_and_stale_safe() {
    for (outcome, expected_kind) in [
        (NativeAttentionSoundOutcome::Played, "delivered"),
        (NativeAttentionSoundOutcome::Unsupported, "unsupported"),
        (NativeAttentionSoundOutcome::Failed, "failed"),
    ] {
        let mut state = ready_state();
        state.native_attention.summary.candidate = Some(NativeAttentionCandidate {
            room_display_name: "Room".to_owned(),
            kind: RoomAttentionKind::Message,
            unread_count: 1,
            highlight_count: 0,
        });
        let current_id = NativeAttentionDispatchId::new(2, 9);
        let stale_same_sequence = NativeAttentionDispatchId::new(1, 9);
        reduce(
            &mut state,
            AppAction::NativeAttentionDispatchStarted {
                dispatch_id: current_id,
            },
        );
        let before = state.clone();
        assert!(
            reduce(
                &mut state,
                AppAction::NativeAttentionDispatchSettled {
                    dispatch_id: stale_same_sequence,
                    outcome,
                }
            )
            .is_empty()
        );
        assert_eq!(state, before);
        reduce(
            &mut state,
            AppAction::NativeAttentionDispatchSettled {
                dispatch_id: current_id,
                outcome,
            },
        );
        assert_eq!(state.native_attention.dispatch.kind(), expected_kind);
    }
}

#[test]
fn native_attention_projection_preserves_and_does_not_replace_active_dispatch() {
    let mut state = ready_state();
    state.native_attention.summary.candidate = Some(NativeAttentionCandidate {
        room_display_name: "Room".to_owned(),
        kind: RoomAttentionKind::Message,
        unread_count: 1,
        highlight_count: 0,
    });
    let active = NativeAttentionDispatchId::new(1, 7);
    let concurrent = NativeAttentionDispatchId::new(2, 7);
    reduce(
        &mut state,
        AppAction::NativeAttentionDispatchStarted {
            dispatch_id: active,
        },
    );
    assert!(
        reduce(
            &mut state,
            AppAction::NativeAttentionDispatchStarted {
                dispatch_id: concurrent,
            },
        )
        .is_empty()
    );

    let mut projection = state.native_attention.clone();
    projection.summary.unread_count = 2;
    projection.dispatch = NativeAttentionDispatchState::Idle;
    reduce(
        &mut state,
        AppAction::NativeAttentionUpdated {
            attention: projection,
        },
    );
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Dispatching {
            dispatch_id: active
        }
    );

    reduce(
        &mut state,
        AppAction::NativeAttentionDispatchSettled {
            dispatch_id: active,
            outcome: NativeAttentionSoundOutcome::Played,
        },
    );
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Delivered {
            dispatch_id: active
        }
    );
}
