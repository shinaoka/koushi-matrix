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

#[test]
fn create_policy_rejections_have_structured_transport_kinds() {
    for (kind, expected) in [
        (
            koushi_protocol::RoomFailureKind::PublicRoomWithRestrictedAccess,
            "publicRoomWithRestrictedAccess",
        ),
        (
            koushi_protocol::RoomFailureKind::ExplicitAccessPolicyWithInvitedOnly,
            "explicitAccessPolicyWithInvitedOnly",
        ),
        (
            koushi_protocol::RoomFailureKind::EmptyAccessPolicyTargets,
            "emptyAccessPolicyTargets",
        ),
    ] {
        let error = CreateRoomInvokeError::from(RequestOutcomeError::OperationFailed {
            failure: koushi_protocol::CoreFailure::RoomOperationFailed { kind },
        });
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({"kind": expected})
        );
    }
}
