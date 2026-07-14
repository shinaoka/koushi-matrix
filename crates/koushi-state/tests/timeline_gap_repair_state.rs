use koushi_state::{
    AppAction, AppState, SessionInfo, SessionState, TimelineContinuityInspection,
    TimelineContinuityState, TimelineGapRepairFailureKind, reduce,
};

const ROOM_ID: &str = "!room:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        }),
        timeline: koushi_state::TimelinePaneState {
            room_id: Some(ROOM_ID.to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn inspection_requires_matching_active_room_and_generation() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::TimelineContinuityInspectionStarted {
            room_id: ROOM_ID.to_owned(),
            generation: 7,
        },
    );
    assert_eq!(
        state.timeline.continuity,
        TimelineContinuityState::Inspecting {
            generation: 7,
            known_gap_count: 0
        }
    );

    reduce(
        &mut state,
        AppAction::TimelineContinuityInspected {
            room_id: ROOM_ID.to_owned(),
            generation: 6,
            inspection: TimelineContinuityInspection::Complete,
        },
    );
    assert!(matches!(
        state.timeline.continuity,
        TimelineContinuityState::Inspecting { generation: 7, .. }
    ));

    reduce(
        &mut state,
        AppAction::TimelineContinuityInspected {
            room_id: ROOM_ID.to_owned(),
            generation: 7,
            inspection: TimelineContinuityInspection::Gapped { gap_count: 2 },
        },
    );
    assert_eq!(
        state.timeline.continuity,
        TimelineContinuityState::Incomplete {
            generation: 7,
            gap_count: 2
        }
    );
}

#[test]
fn only_sdk_complete_marks_timeline_healthy_and_authoritative_start() {
    let mut state = ready_state();
    for (generation, inspection) in [
        (1, TimelineContinuityInspection::Unknown),
        (2, TimelineContinuityInspection::Complete),
    ] {
        reduce(
            &mut state,
            AppAction::TimelineContinuityInspectionStarted {
                room_id: ROOM_ID.to_owned(),
                generation,
            },
        );
        reduce(
            &mut state,
            AppAction::TimelineContinuityInspected {
                room_id: ROOM_ID.to_owned(),
                generation,
                inspection,
            },
        );
    }
    assert_eq!(
        state.timeline.continuity,
        TimelineContinuityState::Healthy {
            generation: 2,
            authoritative_start: true
        }
    );
}

#[test]
fn repair_progress_failure_and_retry_preserve_explicit_incomplete_state() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::TimelineContinuityInspectionStarted {
            room_id: ROOM_ID.to_owned(),
            generation: 3,
        },
    );
    reduce(
        &mut state,
        AppAction::TimelineContinuityInspected {
            room_id: ROOM_ID.to_owned(),
            generation: 3,
            inspection: TimelineContinuityInspection::Gapped { gap_count: 2 },
        },
    );
    reduce(
        &mut state,
        AppAction::TimelineGapRepairStarted {
            room_id: ROOM_ID.to_owned(),
            generation: 4,
            gap_count: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::TimelineGapRepairProgressed {
            room_id: ROOM_ID.to_owned(),
            generation: 4,
            gap_count: 1,
            batches_processed: 2,
        },
    );
    reduce(
        &mut state,
        AppAction::TimelineGapRepairFailed {
            room_id: ROOM_ID.to_owned(),
            generation: 4,
            gap_count: 1,
            batches_processed: 2,
            kind: TimelineGapRepairFailureKind::Network,
        },
    );
    assert_eq!(
        state.timeline.continuity,
        TimelineContinuityState::FailedIncomplete {
            generation: 4,
            gap_count: 1,
            batches_processed: 2,
            failure_kind: TimelineGapRepairFailureKind::Network,
        }
    );

    reduce(
        &mut state,
        AppAction::TimelineGapRepairStarted {
            room_id: ROOM_ID.to_owned(),
            generation: 5,
            gap_count: 1,
        },
    );
    assert_eq!(
        state.timeline.continuity,
        TimelineContinuityState::Repairing {
            generation: 5,
            gap_count: 1,
            batches_processed: 0,
        }
    );
}

#[test]
fn continuity_debug_is_identifier_free() {
    let state = TimelineContinuityState::FailedIncomplete {
        generation: 9,
        gap_count: 1,
        batches_processed: 3,
        failure_kind: TimelineGapRepairFailureKind::Sdk,
    };
    let debug = format!("{state:?}");
    assert!(!debug.contains(ROOM_ID));
    assert!(debug.contains("gap_count"));
}

#[test]
fn replacement_subscription_resets_continuity_generation() {
    let mut state = ready_state();
    state.timeline.continuity = TimelineContinuityState::Healthy {
        generation: 99,
        authoritative_start: true,
    };

    reduce(
        &mut state,
        AppAction::TimelineSubscribed {
            room_id: ROOM_ID.to_owned(),
        },
    );
    assert_eq!(state.timeline.continuity, TimelineContinuityState::Unknown);

    reduce(
        &mut state,
        AppAction::TimelineContinuityInspectionStarted {
            room_id: ROOM_ID.to_owned(),
            generation: 1,
        },
    );
    assert_eq!(
        state.timeline.continuity,
        TimelineContinuityState::Inspecting {
            generation: 1,
            known_gap_count: 0
        }
    );
}
