use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubmissionId(String);

#[derive(Clone, Eq, PartialEq)]
pub enum ComposerSubmissionTarget {
    Main {
        room_id: String,
    },
    Thread {
        room_id: String,
        root_event_id: String,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub enum ComposerSubmissionTerminalOutcome {
    Succeeded,
    Failed { message: String },
    Cancelled,
}

impl SubmissionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SubmissionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SubmissionId(..)")
    }
}
