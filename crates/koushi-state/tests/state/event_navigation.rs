#[path = "navigation_state/support.rs"]
#[allow(
    dead_code,
    reason = "fixture module shared with navigation_state; this binary uses a subset"
)]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
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
    let navigation = koushi_state::NavigationState {
        event_navigation: EventNavigationState::Opening {
            generation: 7,
            source: EventNavigationSource::Search,
        },
        ..Default::default()
    };

    let persisted = navigation.persistence_view();
    let restored: koushi_state::NavigationState =
        serde_json::from_value(serde_json::to_value(&persisted).expect("serialize navigation"))
            .expect("deserialize navigation");

    assert_eq!(restored.event_navigation, EventNavigationState::Idle);
    assert_eq!(persisted.event_navigation, EventNavigationState::Idle);
}

#[test]
fn activity_and_search_missing_targets_fall_back_but_exact_targets_fail() {
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

    for source in [
        EventNavigationSource::Pinned,
        EventNavigationSource::Notification,
        EventNavigationSource::ReplyQuote,
    ] {
        let mut state = ready_navigation_state();
        reduce(&mut state, AppAction::EventNavigationStarted { source });
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
                source: actual,
                failure_kind: EventNavigationFailureKind::TargetMissing,
                ..
            } if *actual == source
        ));
    }
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
    let wire = serde_json::to_value(&state).expect("serialize event navigation");
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

#[test]
fn dismissing_a_navigation_failure_clears_it_without_cancelling_a_live_navigation() {
    // #980: the failure banner must be dismissible, and a stale dismiss must
    // not cancel a navigation that has since been restarted.
    let mut state = ready_navigation_state();

    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Activity,
        },
    );
    let generation = state.navigation.event_navigation.generation();
    reduce(
        &mut state,
        AppAction::EventNavigationFailed {
            generation,
            kind: EventNavigationFailureKind::TargetMissing,
        },
    );
    assert!(matches!(
        &state.navigation.event_navigation,
        EventNavigationState::Failed { .. }
    ));

    reduce(&mut state, AppAction::EventNavigationFailureDismissed);
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Idle
    );

    // A dismiss that arrives after a new navigation started is inert.
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::Pinned,
        },
    );
    let opening = state.navigation.event_navigation.clone();
    reduce(&mut state, AppAction::EventNavigationFailureDismissed);
    assert_eq!(&state.navigation.event_navigation, &opening);

    // Dismissing while idle is a no-op.
    reduce(&mut state, AppAction::EventNavigationFailureDismissed);
    assert_eq!(&state.navigation.event_navigation, &opening);
}

/// #1981: a reply-quote target already shown by the live room timeline settles
/// in place. The terminal carries the target so the renderer can scroll to and
/// emphasize that row, and it obeys the same latest-wins fencing as `Anchored`.
#[test]
fn an_in_place_settle_carries_the_target_and_is_generation_fenced() {
    let mut state = ready_navigation_state();
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::ReplyQuote,
        },
    );
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::ReplyQuote,
        },
    );

    reduce(
        &mut state,
        AppAction::EventNavigationInPlace {
            generation: 1,
            event_id: "$stale-target:example.invalid".to_owned(),
        },
    );
    assert!(
        matches!(
            &state.navigation.event_navigation,
            EventNavigationState::Opening { generation: 2, .. }
        ),
        "a superseded navigation's in-place settle is inert"
    );

    reduce(
        &mut state,
        AppAction::EventNavigationInPlace {
            generation: 2,
            event_id: "$quoted-original:example.invalid".to_owned(),
        },
    );
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::InPlace {
            generation: 2,
            source: EventNavigationSource::ReplyQuote,
            event_id: "$quoted-original:example.invalid".to_owned(),
        }
    );
    assert_eq!(
        state.navigation.main_timeline_anchor, None,
        "an in-place settle never anchors the main pane"
    );

    // A newer intent replaces it like any other terminal.
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::ReplyQuote,
        },
    );
    assert!(matches!(
        &state.navigation.event_navigation,
        EventNavigationState::Opening { generation: 3, .. }
    ));
}

#[test]
fn an_in_place_settle_is_rejected_while_the_main_pane_is_anchored() {
    let mut state = ready_navigation_state();
    state.navigation.active_room_id = Some("room-a".to_owned());
    state.timeline.room_id = Some("room-a".to_owned());
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$anchor:example.invalid".to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());
    reduce(
        &mut state,
        AppAction::EventNavigationStarted {
            source: EventNavigationSource::ReplyQuote,
        },
    );
    let opening = state.navigation.event_navigation.clone();
    reduce(
        &mut state,
        AppAction::EventNavigationInPlace {
            generation: opening.generation(),
            event_id: "$quoted-original:example.invalid".to_owned(),
        },
    );
    assert_eq!(
        &state.navigation.event_navigation, &opening,
        "the live room timeline is not on screen, so there is no row to settle on"
    );
}

#[test]
fn return_to_live_and_room_removal_clear_an_in_place_settle() {
    let settled = || {
        let mut state = ready_navigation_state();
        state.navigation.active_room_id = Some("room-a".to_owned());
        state.timeline.room_id = Some("room-a".to_owned());
        reduce(
            &mut state,
            AppAction::EventNavigationStarted {
                source: EventNavigationSource::ReplyQuote,
            },
        );
        let generation = state.navigation.event_navigation.generation();
        reduce(
            &mut state,
            AppAction::EventNavigationInPlace {
                generation,
                event_id: "$quoted-original:example.invalid".to_owned(),
            },
        );
        assert!(matches!(
            &state.navigation.event_navigation,
            EventNavigationState::InPlace { .. }
        ));
        state
    };

    let mut state = settled();
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

    let mut state = settled();
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

    let mut state = settled();
    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(
        &state.navigation.event_navigation,
        &EventNavigationState::Idle
    );
}

#[test]
fn an_in_place_settle_wire_names_the_target_and_debug_redacts_it() {
    let state = EventNavigationState::InPlace {
        generation: 7,
        source: EventNavigationSource::ReplyQuote,
        event_id: "$quoted-original:example.invalid".to_owned(),
    };
    let wire = serde_json::to_value(&state).expect("serialize event navigation");
    assert_eq!(wire["kind"], "inPlace");
    assert_eq!(wire["source"], "replyQuote");
    assert_eq!(wire["generation"], 7);
    assert_eq!(wire["eventId"], "$quoted-original:example.invalid");
    let debug = format!("{state:?}");
    assert!(
        !debug.contains("quoted-original"),
        "Debug must not print the event id: {debug}"
    );
}
