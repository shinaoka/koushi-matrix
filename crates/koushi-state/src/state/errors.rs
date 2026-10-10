use serde::{Deserialize, Serialize};

use super::AuthFailureKind;

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
    /// Bounded, Rust-classified reason for the visible guidance when the error
    /// carries one (#1268). Absent for legacy/generic errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<AuthFailureKind>,
}

impl AppError {
    /// Build a generic error with no classified reason.
    pub fn new(code: impl Into<String>, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            recoverable,
            reason: None,
        }
    }
}
