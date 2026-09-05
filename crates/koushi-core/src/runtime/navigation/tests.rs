use super::*;
use crate::timeline::FocusedProjectionCommitted;
use koushi_protocol::ids::{AccountKey, RuntimeConnectionId, TimelineGeneration};
use koushi_state::SessionInfo;

fn focused_projection_fixture(sequence: u64) -> PendingFocusedNavigation {
    PendingFocusedNavigation {
        projection_request_id: RequestId {
            connection_id: RuntimeConnectionId(3),
            sequence,
        },
        key: TimelineKey {
            account_key: AccountKey("@qa:example.invalid".to_owned()),
            kind: TimelineKind::Focused {
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$target".to_owned(),
            },
        },
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$target".to_owned(),
        allow_live_fallback: true,
        generation: None,
    }
}

fn focused_projection_commit(
    pending: &PendingFocusedNavigation,
    actor_generation: u64,
    target_present: bool,
) -> FocusedProjectionCommitted {
    FocusedProjectionCommitted {
        projection_request_id: pending.projection_request_id,
        key: pending.key.clone(),
        actor_generation,
        timeline_generation: TimelineGeneration(0),
        item_count: u64::from(target_present),
        target_present,
    }
}

#[test]
fn focused_projection_commit_settles_without_renderer_evidence() {
    let expected = focused_projection_fixture(20);
    let mut pending = Some(expected.clone());
    let commit = focused_projection_commit(&expected, 41, true);

    assert_eq!(
        focused_navigation_action_after_projection_commit(&mut pending, &commit),
        Some(AppAction::EnterAnchoredTimeline {
            room_id: expected.room_id,
            event_id: expected.event_id,
        })
    );
    assert!(pending.is_none());
}

#[test]
fn focused_projection_commit_missing_target_uses_live_fallback_policy() {
    let expected = focused_projection_fixture(21);
    let mut pending = Some(expected.clone());
    let commit = focused_projection_commit(&expected, 41, false);

    assert_eq!(
        focused_navigation_action_after_projection_commit(&mut pending, &commit),
        Some(AppAction::CloseFocusedContext)
    );
    assert!(pending.is_none());

    let mut pinned = expected.clone();
    pinned.allow_live_fallback = false;
    let mut pending = Some(pinned.clone());
    let commit = focused_projection_commit(&pinned, 41, false);
    assert_eq!(
        focused_navigation_action_after_projection_commit(&mut pending, &commit),
        Some(AppAction::CloseFocusedContext)
    );
}

#[test]
fn stale_or_reordered_focused_projection_commits_are_inert() {
    let expected = focused_projection_fixture(22);
    let mut pending = Some(expected.clone());

    let mut latest =
        std::collections::HashMap::from([(expected.key.clone(), (41, TimelineGeneration(0)))]);
    let mut stale_generation = focused_projection_commit(&expected, 40, true);
    assert!(!admit_focused_projection_generation(
        &mut latest,
        &stale_generation
    ));
    assert_eq!(pending, Some(expected.clone()));

    stale_generation.key =
        TimelineKey::room(expected.key.account_key.clone(), "!other:example.invalid");
    assert!(
        focused_navigation_action_after_projection_commit(&mut pending, &stale_generation)
            .is_none()
    );
    assert_eq!(pending, Some(expected.clone()));

    let newer = focused_projection_commit(&expected, 41, true);
    assert!(admit_focused_projection_generation(&mut latest, &newer));
    assert!(focused_navigation_action_after_projection_commit(&mut pending, &newer).is_some());
    assert!(pending.is_none());
    assert!(
        focused_navigation_action_after_projection_commit(&mut pending, &stale_generation)
            .is_none()
    );
}
#[test]
fn focused_navigation_lifecycle_uses_the_reduced_state() {
    let expected = focused_projection_fixture(13);
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        focused_context: FocusedContextState::Open {
            room_id: expected.room_id.clone(),
            event_id: expected.event_id.clone(),
            is_subscribed: true,
        },
        ..AppState::default()
    };
    state.navigation.active_room_id = Some(expected.room_id.clone());
    state.navigation.main_timeline_anchor = Some(koushi_state::MainTimelineAnchor {
        event_id: expected.event_id.clone(),
    });
    assert_eq!(
        focused_navigation_outcome_after_reduce(&state, &expected, true),
        IntentOutcome::Committed
    );

    state.navigation.main_timeline_anchor = None;
    state.focused_context = FocusedContextState::Closed;
    assert_eq!(
        focused_navigation_outcome_after_reduce(&state, &expected, false),
        IntentOutcome::BenignNoOp(IntentNoOpReason::TimelineTargetMissing)
    );

    let mut pinned_navigation = expected.clone();
    pinned_navigation.allow_live_fallback = false;
    assert_eq!(
        focused_navigation_outcome_after_reduce(&state, &pinned_navigation, false),
        IntentOutcome::FailedNoOp(IntentNoOpReason::TimelineTargetMissing)
    );

    state.navigation.active_room_id = Some("!other:example.invalid".to_owned());
    assert_eq!(
        focused_navigation_outcome_after_reduce(&state, &expected, true),
        IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState)
    );
}
#[test]
fn replacement_focused_helper_preserves_same_key_and_unsubscribes_different_key() {
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let current = TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Focused {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event-a:example.invalid".to_owned(),
        },
    };
    let same = current.clone();
    let different = TimelineKey {
        account_key,
        kind: TimelineKind::Focused {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event-b:example.invalid".to_owned(),
        },
    };

    assert_eq!(
        unsubscribe_replaced_focused_context_timeline_key(Some(current.clone()), same),
        None
    );
    assert_eq!(
        unsubscribe_replaced_focused_context_timeline_key(Some(current.clone()), different),
        Some(current)
    );
    assert_eq!(
        unsubscribe_replaced_focused_context_timeline_key(
            None,
            focused_key("$event-c:example.invalid")
        ),
        None
    );
}
#[test]
fn select_space_cleanup_targets_previous_room_only_when_active_room_changes() {
    let action = AppAction::SelectSpace {
        space_id: Some("!space:example.invalid".to_owned()),
    };

    assert_eq!(
        navigation_replacement_room_for_cleanup(
            &action,
            Some("!old:example.invalid"),
            Some("!next:example.invalid"),
        ),
        Some(NavigationReplacementRoomForCleanup::Room(
            "!next:example.invalid".to_owned()
        ))
    );
    assert_eq!(
        navigation_replacement_room_for_cleanup(&action, Some("!old:example.invalid"), None,),
        Some(NavigationReplacementRoomForCleanup::Cleared)
    );
    assert_eq!(
        navigation_replacement_room_for_cleanup(
            &action,
            Some("!same:example.invalid"),
            Some("!same:example.invalid"),
        ),
        None
    );
    assert_eq!(
        navigation_replacement_room_for_cleanup(&action, None, None),
        None
    );
}
#[test]
fn select_room_cleanup_still_uses_explicit_target_room() {
    let action = AppAction::SelectRoom {
        room_id: "!target:example.invalid".to_owned(),
    };

    assert_eq!(
        navigation_replacement_room_for_cleanup(
            &action,
            Some("!old:example.invalid"),
            Some("!target:example.invalid"),
        ),
        Some(NavigationReplacementRoomForCleanup::Room(
            "!target:example.invalid".to_owned()
        ))
    );
}
#[test]
fn navigation_preference_boundary_rejects_invalid_and_oversized_imports() {
    assert!(
        normalize_navigation_preference_update(NavigationPreferenceUpdate::SetSpacePresentation {
            space_id: "not-a-matrix-id".to_owned(),
            presentation: Some(SpaceLocalPresentation {
                name: Some("Private".to_owned()),
                icon: None,
            }),
        })
        .is_err()
    );

    let oversized = (0..=MAX_SPACE_LOCAL_PRESENTATIONS)
        .map(|index| {
            (
                format!("!space-{index}:example.invalid"),
                SpaceLocalPresentation {
                    name: Some(format!("Space {index}")),
                    icon: None,
                },
            )
        })
        .collect();
    assert!(
        normalize_navigation_preference_update(NavigationPreferenceUpdate::ImportLegacy {
            home_selection: None,
            space_local_presentations: SpaceLocalPresentations(oversized),
        })
        .is_err()
    );

    let full = NavigationState {
        space_local_presentations: SpaceLocalPresentations(
            (0..MAX_SPACE_LOCAL_PRESENTATIONS)
                .map(|index| {
                    (
                        format!("!space-{index}:example.invalid"),
                        SpaceLocalPresentation {
                            name: Some(format!("Space {index}")),
                            icon: None,
                        },
                    )
                })
                .collect(),
        ),
        ..NavigationState::default()
    };
    assert!(navigation_preference_exceeds_capacity(
        &full,
        &NavigationPreferenceUpdate::SetSpacePresentation {
            space_id: "!one-more:example.invalid".to_owned(),
            presentation: Some(SpaceLocalPresentation {
                name: Some("One more".to_owned()),
                icon: None,
            }),
        }
    ));
}

#[test]
fn navigation_preference_boundary_canonicalizes_empty_presentations() {
    let update =
        normalize_navigation_preference_update(NavigationPreferenceUpdate::SetSpacePresentation {
            space_id: "!space:example.invalid".to_owned(),
            presentation: Some(SpaceLocalPresentation {
                name: Some("   ".to_owned()),
                icon: None,
            }),
        })
        .expect("valid preference update");
    assert!(matches!(
        update,
        NavigationPreferenceUpdate::SetSpacePresentation {
            presentation: None,
            ..
        }
    ));
}

fn focused_key(event_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@alice:example.invalid".to_owned()),
        kind: TimelineKind::Focused {
            room_id: "!room:example.invalid".to_owned(),
            event_id: event_id.to_owned(),
        },
    }
}
