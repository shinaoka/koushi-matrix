use super::room_management_forbidden_recorded;
use crate::registry::{QaScenario, QaStage, final_tokens_for_scenario};
use crate::{
    AppState, OperationFailureKind, RequestId, RoomManagementOperationKind,
    RoomManagementOperationState,
};

#[test]
fn room_management_scenario_runs_after_room_space_and_reports_private_tokens() {
    assert!(QaScenario::RoomManagement.should_run_stage(QaStage::Safety));
    assert!(QaScenario::RoomManagement.should_run_stage(QaStage::LoginSync));
    assert!(QaScenario::RoomManagement.should_run_stage(QaStage::RoomSpace));
    assert!(QaScenario::RoomManagement.should_run_stage(QaStage::RoomManagement));
    assert!(!QaScenario::RoomManagement.should_run_stage(QaStage::Timeline));
    assert!(QaScenario::RoomManagement.suppress_matrix_identifiers());

    assert_eq!(
        final_tokens_for_scenario(QaScenario::RoomManagement),
        [
            "safety=ok",
            "login_sync=ok",
            "room_space=ok",
            "room_settings=ok",
            "moderation=ok",
            "permission_guard=ok",
            "space_access=ok",
            "space_add_existing=ok",
            "restore_cleanup=ok",
        ]
    );
}

#[test]
fn room_management_forbidden_predicate_requires_matching_failed_moderation_state() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 42,
    };
    let mut state = AppState::default();

    assert!(!room_management_forbidden_recorded(
        &state,
        request_id,
        RoomManagementOperationKind::Moderation
    ));

    state.room_management.operation = RoomManagementOperationState::Failed {
        request_id: 41,
        room_id: "!redacted:example.invalid".to_owned(),
        operation: RoomManagementOperationKind::Moderation,
        kind: OperationFailureKind::Forbidden,
    };
    assert!(!room_management_forbidden_recorded(
        &state,
        request_id,
        RoomManagementOperationKind::Moderation
    ));

    state.room_management.operation = RoomManagementOperationState::Failed {
        request_id: 42,
        room_id: "!redacted:example.invalid".to_owned(),
        operation: RoomManagementOperationKind::Moderation,
        kind: OperationFailureKind::Forbidden,
    };
    assert!(room_management_forbidden_recorded(
        &state,
        request_id,
        RoomManagementOperationKind::Moderation
    ));
}

#[test]
fn room_management_forbidden_predicate_distinguishes_the_operation() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 42,
    };
    let mut state = AppState::default();
    state.room_management.operation = RoomManagementOperationState::Failed {
        request_id: 42,
        room_id: "!redacted:example.invalid".to_owned(),
        operation: RoomManagementOperationKind::Settings,
        kind: OperationFailureKind::Forbidden,
    };

    assert!(room_management_forbidden_recorded(
        &state,
        request_id,
        RoomManagementOperationKind::Settings
    ));
    assert!(!room_management_forbidden_recorded(
        &state,
        request_id,
        RoomManagementOperationKind::Moderation
    ));
}
