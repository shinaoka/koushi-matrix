use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationFailureKind {
    Forbidden,
    NotFound,
    Network,
    Timeout,
    Invalid,
    Sdk,
    /// The current join-rule content has allow conditions this client does not
    /// model, so rewriting it would drop them (#1177).
    UnsupportedPolicyCondition,
    /// The current join-rule policy could not be read from the store before
    /// the write, so the edit was not attempted (#1177).
    PolicyNotVerified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}
