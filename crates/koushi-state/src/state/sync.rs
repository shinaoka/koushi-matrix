use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncState {
    Stopped,
    Starting,
    Running,
    Failed { reason: String },
    Reconnecting { reason: String },
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncMode {
    #[default]
    Unsupported,
    Legacy,
    Simplified,
    Transitioning,
    Failed {
        #[serde(rename = "failureKind")]
        kind: SyncModeFailureKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncModeFailureKind {
    Network,
    Auth,
    Store,
    Internal,
}
