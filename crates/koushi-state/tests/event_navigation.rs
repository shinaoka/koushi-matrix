#[path = "navigation_state/support.rs"]
mod support;

use koushi_state::{
    AppAction, EventNavigationFailureKind, EventNavigationSource, EventNavigationState, reduce,
};
use support::{ready_state, rooms, spaces};

fn ready_navigation_state() -> koushi_state::AppState {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    state
}

#[test]
fn event_navigation_is_latest_wins_and_stale_settlements_are_inert() {
    let mut state = ready_navigation_state();

    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Activity,
        },
    );
    assert_eq!(state.navigation.event_navigation.generation(), 1);

    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Search,
        },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Opening {
            generation: 2,
            source: EventNavigationSource::Search,
        }
    );

    for stale in [
        AppAction::EventNavigationAnchored { generation: 1 },
        AppAction::EventNavigationLiveFallback { generation: 1 },
        AppAction::EventNavigationFailed {
            generation: 1,
            kind: EventNavigationFailureKind::Timeline,
        },
    ] {
        reduce(&mut state, stale);
        assert!(matches!(
            &state.navigation.event_navigation,
            EventNavigationState::Opening { generation: 2, .. }
        ));
    }

    reduce(
        &mut state,
        AppAction::EventNavigationAnchored { generation: 2 },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Anchored {
            generation: 2,
            source: EventNavigationSource::Search,
        }
    );
}

#[test]
fn current_missing_target_uses_rust_owned_source_policy_and_new_intent_clears_failure() {
    let mut state = ready_navigation_state();
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Pinned,
        },
    );
    reduce(
        &mut state,
        AppAction::EventNavigationFailed {
            generation: 1,
            kind: EventNavigationFailureKind::TargetMissing,
        },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Failed {
            generation: 1,
            source: EventNavigationSource::Pinned,
            failure_kind: EventNavigationFailureKind::TargetMissing,
        }
    );

    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Activity,
        },
    );
    reduce(
        &mut state,
        AppAction::EventNavigationLiveFallback { generation: 2 },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::LiveFallback {
            generation: 2,
            source: EventNavigationSource::Activity,
        }
    );
}

#[test]
fn event_navigation_is_transient_when_navigation_is_persisted() {
    let mut navigation = koushi_state::NavigationState::default();
    navigation.event_navigation = EventNavigationState::Opening {
        generation: 7,
        source: EventNavigationSource::Search,
    };

    let persisted = navigation.persistence_view();
    let restored: koushi_state::NavigationState =
        serde_json::from_value(serde_json::to_value(&persisted).expect("serialize navigation"))
            .expect("deserialize navigation");

    assert_eq!(restored.event_navigation, EventNavigationState::Idle);
    assert_eq!(persisted.event_navigation, EventNavigationState::Idle);
}

#[test]
fn activity_and_search_missing_targets_fall_back_but_pinned_fails() {
    for source in [
        EventNavigationSource::Activity,
        EventNavigationSource::Search,
    ] {
        let mut state = ready_navigation_state();
        reduce(&mut state, AppAction::EventNavigationStarted { source });
        reduce(
            &mut state,
            AppAction::EventNavigationLiveFallback { generation: 1 },
        );
        assert!(matches!(
            &state.navigation.event_navigation,
            EventNavigationState::LiveFallback { source: actual, .. } if *actual == source
        ));
    }

    let mut state = ready_navigation_state();
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Pinned,
        },
    );
    reduce(
        &mut state,
        AppAction::EventNavigationFailed {
            generation: 1,
            kind: EventNavigationFailureKind::TargetMissing,
        },
    );
    assert!(matches!(
        &state.navigation.event_navigation,
        EventNavigationState::Failed {
            source: EventNavigationSource::Pinned,
            failure_kind: EventNavigationFailureKind::TargetMissing,
            ..
        }
    ));
}

#[test]
fn navigation_loaded_discards_transient_event_navigation() {
    let mut state = ready_navigation_state();
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Activity,
        },
    );
    let navigation = state.navigation.clone();
    reduce(&mut state, AppAction::NavigationLoaded { navigation });
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Idle
    );
}

#[test]
fn room_removal_clears_event_navigation() {
    let mut state = ready_navigation_state();
    state.navigation.active_room_id = Some("room-a".to_owned());
    state.timeline.room_id = Some("room-a".to_owned());
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Search,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: Vec::new(),
        },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Idle
    );
}

#[test]
fn event_navigation_wire_and_debug_are_private_safe() {
    let state = EventNavigationState::Failed {
        generation: 4,
        source: EventNavigationSource::Activity,
        failure_kind: EventNavigationFailureKind::TargetMissing,
    };
    let wire = serde_json::to_value(state).expect("serialize event navigation");
    assert_eq!(wire["kind"], "failed");
    assert_eq!(wire["source"], "activity");
    assert_eq!(wire["failureKind"], "targetMissing");
    assert!(!format!("{state:?}").contains("event_id"));
}

#[test]
fn return_to_live_and_session_exit_clear_event_navigation() {
    let mut state = ready_navigation_state();
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Activity,
        },
    );
    reduce(
        &mut state,
        AppAction::ReturnMainTimelineToLive {
            room_id: "room-a".to_owned(),
        },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Idle
    );

    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Activity,
        },
    );
    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Idle
    );
}
