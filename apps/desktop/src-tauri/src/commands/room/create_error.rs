use super::*;

#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CreateRoomInvokeError {
    AliasInUse,
    Failed { message: String },
}

impl From<RequestOutcomeError> for CreateRoomInvokeError {
    fn from(error: RequestOutcomeError) -> Self {
        match error {
            RequestOutcomeError::OperationFailed {
                failure:
                    koushi_protocol::CoreFailure::RoomOperationFailed {
                        kind: koushi_protocol::RoomFailureKind::AliasInUse,
                    },
            } => Self::AliasInUse,
            error => Self::Failed {
                message: invoke_error_from_request_outcome("room creation", error),
            },
        }
    }
}

#[cfg(test)]
#[path = "create_error_tests.rs"]
mod tests;
