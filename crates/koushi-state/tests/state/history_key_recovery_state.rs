use koushi_state::{
    AppAction, AppState, HistoryKeyRecoveryState, HistoryKeyRecoveryStatus,
    history_key_request_targets_all, reduce,
};

#[test]
fn broad_default_requires_a_complete_small_population_and_is_never_automatic() {
    assert!(history_key_request_targets_all(Some(30), false, false));
    assert!(!history_key_request_targets_all(Some(31), false, false));
    assert!(!history_key_request_targets_all(None, false, false));
    assert!(history_key_request_targets_all(Some(31), true, false));
    assert!(!history_key_request_targets_all(Some(2), true, true));
}

#[test]
fn retry_fences_stale_completions_and_other_rooms() {
    let mut state = AppState::default();
    for (generation, status) in [
        (2, HistoryKeyRecoveryStatus::Sending),
        (
            1,
            HistoryKeyRecoveryStatus::Requested {
                recipients: 1,
                responses: 1,
                received_keys: 2,
            },
        ),
    ] {
        reduce(
            &mut state,
            AppAction::HistoryKeyRecoveryChanged {
                room_id: "!history:example.invalid".to_owned(),
                state: HistoryKeyRecoveryState { generation, status },
            },
        );
    }
    assert_eq!(
        state.room_management.history_keys["!history:example.invalid"].generation,
        2
    );
    assert_eq!(
        state.room_management.history_keys["!history:example.invalid"].status,
        HistoryKeyRecoveryStatus::Sending
    );
    assert!(
        !state
            .room_management
            .history_keys
            .contains_key("!other:example.invalid")
    );
}

#[test]
fn logout_erases_recovery_projection_and_debug_never_exposes_room_identity() {
    let mut state = AppState::default();
    reduce(
        &mut state,
        AppAction::HistoryKeyRecoveryChanged {
            room_id: "!private-history:example.invalid".into(),
            state: HistoryKeyRecoveryState {
                generation: 4,
                status: HistoryKeyRecoveryStatus::Failed {
                    reason: koushi_state::HistoryKeyRecoveryFailure::Network,
                },
            },
        },
    );
    assert!(!format!("{:?}", state.room_management).contains("private-history"));
    reduce(&mut state, AppAction::LogoutFinished);
    assert!(state.room_management.history_keys.is_empty());
}

#[test]
fn a_delayed_sending_observation_cannot_regress_the_same_operation() {
    let mut state = AppState::default();
    for status in [
        HistoryKeyRecoveryStatus::Requested {
            recipients: 1,
            responses: 1,
            received_keys: 2,
        },
        HistoryKeyRecoveryStatus::Sending,
    ] {
        reduce(
            &mut state,
            AppAction::HistoryKeyRecoveryChanged {
                room_id: "!history:example.invalid".into(),
                state: HistoryKeyRecoveryState {
                    generation: 1,
                    status,
                },
            },
        );
    }
    assert!(matches!(
        state.room_management.history_keys["!history:example.invalid"].status,
        HistoryKeyRecoveryStatus::Requested {
            received_keys: 2,
            ..
        }
    ));
}
