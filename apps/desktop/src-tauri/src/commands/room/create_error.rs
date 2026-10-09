use super::*;

#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CreateRoomInvokeError {
    AliasInUse,
    /// #1177: the create request combined public visibility with an explicit
    /// restricted access policy.
    PublicRoomWithRestrictedAccess,
    /// #1177: the create request combined an explicit access policy with
    /// `invitedOnly`.
    ExplicitAccessPolicyWithInvitedOnly,
    /// #1177: the create request selected a restricted access policy without a
    /// single membership allow target.
    EmptyAccessPolicyTargets,
    Failed {
        message: String,
    },
}

impl From<RequestOutcomeError> for CreateRoomInvokeError {
    fn from(error: RequestOutcomeError) -> Self {
        use koushi_protocol::{CoreFailure, RoomFailureKind};
        if let RequestOutcomeError::OperationFailed {
            failure: CoreFailure::RoomOperationFailed { kind },
        } = &error
        {
            match kind {
                RoomFailureKind::AliasInUse => return Self::AliasInUse,
                RoomFailureKind::PublicRoomWithRestrictedAccess => {
                    return Self::PublicRoomWithRestrictedAccess;
                }
                RoomFailureKind::ExplicitAccessPolicyWithInvitedOnly => {
                    return Self::ExplicitAccessPolicyWithInvitedOnly;
                }
                RoomFailureKind::EmptyAccessPolicyTargets => {
                    return Self::EmptyAccessPolicyTargets;
                }
                _ => {}
            }
        }
        Self::Failed {
            message: invoke_error_from_request_outcome("room creation", error),
        }
    }
}

#[cfg(test)]
#[path = "create_error_tests.rs"]
mod tests;
