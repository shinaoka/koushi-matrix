use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalEncryptionState {
    #[default]
    Unknown,
    Probing {
        request_id: u64,
    },
    Healthy,
    Unavailable,
    LockedOrInaccessible,
    MissingCredential,
    ResetRequired,
    Resetting {
        request_id: u64,
    },
}

impl LocalEncryptionState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Probing { .. } => "probing",
            Self::Healthy => "healthy",
            Self::Unavailable => "unavailable",
            Self::LockedOrInaccessible => "locked_or_inaccessible",
            Self::MissingCredential => "missing_credential",
            Self::ResetRequired => "reset_required",
            Self::Resetting { .. } => "resetting",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalEncryptionHealth {
    Unknown,
    Healthy,
    Unavailable,
    LockedOrInaccessible,
    MissingCredential,
    ResetRequired,
}

impl From<LocalEncryptionHealth> for LocalEncryptionState {
    fn from(health: LocalEncryptionHealth) -> Self {
        match health {
            LocalEncryptionHealth::Unknown => Self::Unknown,
            LocalEncryptionHealth::Healthy => Self::Healthy,
            LocalEncryptionHealth::Unavailable => Self::Unavailable,
            LocalEncryptionHealth::LockedOrInaccessible => Self::LockedOrInaccessible,
            LocalEncryptionHealth::MissingCredential => Self::MissingCredential,
            LocalEncryptionHealth::ResetRequired => Self::ResetRequired,
        }
    }
}
