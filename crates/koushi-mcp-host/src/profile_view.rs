//! What the settings page renders about agent profiles (#2132, design section
//! 8). Rust decides every fact here: React draws it and computes none of it.
//! The secret is never part of a view; only the path of the file that holds it.

use serde::Serialize;

use crate::profiles::ProfileGrants;
use crate::registration::Registration;

/// Whether a profile can be launched right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Ready,
    /// The bound account is not signed in on this host.
    AccountUnavailable,
    /// The bound account lost its agent role.
    NotAgentAccount,
    /// The row exists but its secret file is gone, so no plugin can start.
    SecretMissing,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct ProfileView {
    pub profile_id: String,
    pub label: String,
    pub account_id: String,
    pub read: bool,
    pub send: bool,
    pub attachments: bool,
    pub message_actions: bool,
    pub draft: bool,
    pub status: ProfileStatus,
    /// At least one live profiled connection.
    pub connected: bool,
    /// The profile's push-subscription lease is held.
    pub subscribed: bool,
    /// When a connection of this profile last came or went, or its lease changed.
    pub last_seen_ms: Option<u64>,
    /// When a connection last presented the wrong secret for this profile.
    pub last_rejected_ms: Option<u64>,
    /// Launch snippets per client, built in Rust; empty without a launcher.
    pub snippets: Vec<Registration>,
}

impl std::fmt::Debug for ProfileView {
    // Labels, account ids and paths stay out of logs.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProfileView")
            .field("profile_id", &self.profile_id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// The stored facts of one profile, before the live state is merged in.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ProfileFacts {
    pub(crate) profile_id: String,
    pub(crate) label: String,
    pub(crate) account_id: String,
    pub(crate) grants: ProfileGrants,
    pub(crate) status: ProfileStatus,
    pub(crate) secret_path: String,
    pub(crate) last_rejected_ms: Option<u64>,
}

/// Connections and lease of one profile, kept in memory only.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Live {
    pub(crate) connections: usize,
    pub(crate) subscribed: bool,
    pub(crate) last_seen_ms: Option<u64>,
}
