use super::*;
#[test]
fn alias_collision_has_a_structured_transport_kind() {
    let error = CreateRoomInvokeError::from(RequestOutcomeError::OperationFailed {
        failure: koushi_protocol::CoreFailure::RoomOperationFailed {
            kind: koushi_protocol::RoomFailureKind::AliasInUse,
        },
    });
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({"kind":"aliasInUse"})
    );
}
