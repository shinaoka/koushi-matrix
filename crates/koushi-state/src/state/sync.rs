use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncState {
    Stopped,
    Starting,
    Running,
    Failed { reason: String },
    Reconnecting { reason: String },
}

// #1985: `reason` is whatever the transport reported, so it can carry
// identities or server text; a diagnostic dump reports the state, not the text.
impl std::fmt::Debug for SyncState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => formatter.write_str("Stopped"),
            Self::Starting => formatter.write_str("Starting"),
            Self::Running => formatter.write_str("Running"),
            Self::Failed { .. } => formatter
                .debug_struct("Failed")
                .field("reason", &"Reason(..)")
                .finish(),
            Self::Reconnecting { .. } => formatter
                .debug_struct("Reconnecting")
                .field("reason", &"Reason(..)")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncLifecycleStatus {
    Stopped,
    Starting,
    Running,
    Failed { reason: String },
    Reconnecting { reason: String },
}
