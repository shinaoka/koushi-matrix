use std::fmt;

use serde::{Deserialize, Serialize};

use super::{CurrentDeviceTrustState, SessionAuthenticationMethod};

/// #982/#1009: a successful check stays authoritative for this long, so
/// reopening the session-status panel reads state instead of re-running a full
/// remote inspection (account devices, own identity, crypto device, backup
/// probe). It is also the period of the low-frequency automatic recheck.
pub const SESSION_STATUS_FRESHNESS_MS: u64 = 6 * 60 * 60 * 1_000;

/// Cooldown after the first failed check. Doubles per consecutive failure up to
/// [`SESSION_STATUS_FAILURE_BACKOFF_CAP_MS`].
pub const SESSION_STATUS_FAILURE_BACKOFF_BASE_MS: u64 = 60 * 1_000;

/// Upper bound on the failure cooldown. There is no retry ceiling: automatic
/// checks keep retrying at this interval after repeated failures (#1009).
pub const SESSION_STATUS_FAILURE_BACKOFF_CAP_MS: u64 = 30 * 60 * 1_000;

/// Request ids minted by the reducer for automatic (`Scheduled`/`Recovery`)
/// checks carry this bit, disjoint from per-connection command sequences.
pub const SESSION_STATUS_SCHEDULED_REQUEST_ID_BASE: u64 = 1 << 62;

/// Cooldown before the next automatic check, given how many consecutive
/// failures preceded it.
pub fn session_status_failure_backoff_ms(consecutive_failures: u32) -> u64 {
    let exponent = consecutive_failures.saturating_sub(1).min(16);
    SESSION_STATUS_FAILURE_BACKOFF_BASE_MS
        .saturating_mul(1u64 << exponent)
        .min(SESSION_STATUS_FAILURE_BACKOFF_CAP_MS)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatusRefreshTrigger {
    Open,
    Manual,
    /// Core-owned: a due time armed after a settlement, or for a session that
    /// has not been checked yet, elapsed.
    Scheduled,
    /// Core-owned: a due time re-armed by a sync connectivity recovery edge
    /// elapsed.
    Recovery,
}

impl SessionStatusRefreshTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::Recovery => "recovery",
        }
    }

    /// Only these triggers may be submitted through the frontend command
    /// surface; the others are minted by the reducer's scheduler.
    pub fn is_frontend_submittable(self) -> bool {
        matches!(self, Self::Open | Self::Manual)
    }
}

/// The reducer's decision for one check trigger, recorded for the
/// private-safe diagnostic summary (#1009).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStatusCheckDecision {
    /// A network check was admitted.
    Started,
    /// A check was already in flight; the trigger joined it.
    Joined,
    /// Not due yet, or no Ready session: the last status is served.
    Deferred,
    /// A due notification arrived while sync connectivity was unproven; the
    /// next connectivity edge re-arms it.
    Dropped,
}

impl SessionStatusCheckDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Joined => "joined",
            Self::Deferred => "deferred",
            Self::Dropped => "dropped",
        }
    }
}

/// Cumulative decision counts. They survive session resets so a repeated
/// check path stays traceable after the diagnostic ring discards old records.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionStatusCheckStats {
    pub started: u64,
    pub joined: u64,
    pub deferred: u64,
    pub dropped: u64,
    pub succeeded: u64,
    pub failed: u64,
}

/// One armed due time, owned by the reducer and executed by a Core timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArmedSessionStatusCheck {
    pub token: u64,
    pub due_at_ms: u64,
    pub trigger: SessionStatusRefreshTrigger,
}

/// #1009: the single owner of session-status check scheduling. It is not part
/// of the rendered snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionStatusSchedule {
    /// Latest issued token. Every arm and every session reset advances it, so
    /// a timer notification carrying an older token is inert.
    pub token: u64,
    pub armed: Option<ArmedSessionStatusCheck>,
    pub next_request_seq: u64,
    pub stats: SessionStatusCheckStats,
    pub last_trigger: Option<SessionStatusRefreshTrigger>,
    pub last_decision: Option<SessionStatusCheckDecision>,
}

impl SessionStatusSchedule {
    /// Fence every outstanding due notification (session teardown or reset).
    pub fn retire(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.armed = None;
    }

    pub(crate) fn next_request_id(&mut self) -> u64 {
        self.next_request_seq = self.next_request_seq.wrapping_add(1);
        SESSION_STATUS_SCHEDULED_REQUEST_ID_BASE
            | (self.next_request_seq & (SESSION_STATUS_SCHEDULED_REQUEST_ID_BASE - 1))
    }

    pub(crate) fn record(
        &mut self,
        trigger: SessionStatusRefreshTrigger,
        decision: SessionStatusCheckDecision,
    ) {
        let counter = match decision {
            SessionStatusCheckDecision::Started => &mut self.stats.started,
            SessionStatusCheckDecision::Joined => &mut self.stats.joined,
            SessionStatusCheckDecision::Deferred => &mut self.stats.deferred,
            SessionStatusCheckDecision::Dropped => &mut self.stats.dropped,
        };
        *counter = counter.saturating_add(1);
        self.last_trigger = Some(trigger);
        self.last_decision = Some(decision);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSessionSyncState {
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnIdentityVerification {
    Missing,
    Unverified,
    Verified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSessionBackupState {
    Ready,
    Disabled,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSessionStatusFailureKind {
    Sdk,
    TimedOut,
    Unavailable,
    ConnectivityUnavailable,
    Authentication,
    Network,
    Server,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CurrentSessionStatusDetails {
    pub device_display_name: Option<String>,
    pub device_id: String,
    pub authentication_method: SessionAuthenticationMethod,
    pub sync_state: CurrentSessionSyncState,
    pub is_cross_signed_by_owner: bool,
    pub own_identity_verification: OwnIdentityVerification,
    pub key_backup: CurrentSessionBackupState,
    pub verification: CurrentDeviceTrustState,
    pub checked_at_ms: u64,
}

impl CurrentSessionStatusDetails {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device_display_name: Option<String>,
        device_id: String,
        authentication_method: SessionAuthenticationMethod,
        sync_state: CurrentSessionSyncState,
        verification: CurrentDeviceTrustState,
        is_cross_signed_by_owner: bool,
        own_identity_verification: OwnIdentityVerification,
        key_backup: CurrentSessionBackupState,
        checked_at_ms: u64,
    ) -> Self {
        Self {
            device_display_name,
            device_id,
            authentication_method,
            sync_state,
            is_cross_signed_by_owner,
            own_identity_verification,
            key_backup,
            verification,
            checked_at_ms,
        }
    }
}

impl fmt::Debug for CurrentSessionStatusDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentSessionStatusDetails")
            .field(
                "device_display_name",
                &self.device_display_name.as_ref().map(|_| "DeviceName(..)"),
            )
            .field("device_id", &"DeviceId(..)")
            .field("authentication_method", &self.authentication_method)
            .field("sync_state", &self.sync_state)
            .field("is_cross_signed_by_owner", &self.is_cross_signed_by_owner)
            .field("own_identity_verification", &self.own_identity_verification)
            .field("key_backup", &self.key_backup)
            .field("verification", &self.verification)
            .field("checked_at_ms", &self.checked_at_ms)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CurrentSessionStatusState {
    #[default]
    Idle,
    Checking {
        request_id: u64,
        trigger: SessionStatusRefreshTrigger,
        #[serde(default)]
        last_known_details: Option<CurrentSessionStatusDetails>,
        /// Failures preceding this attempt, carried so the backoff survives the
        /// round trip through `Checking` (#982).
        #[serde(default)]
        consecutive_failures: u32,
    },
    Ready {
        request_id: u64,
        details: CurrentSessionStatusDetails,
    },
    Failed {
        request_id: u64,
        kind: CurrentSessionStatusFailureKind,
        checked_at_ms: u64,
        #[serde(default)]
        last_known_details: Option<CurrentSessionStatusDetails>,
        /// Consecutive failed checks including this one; drives the backoff.
        #[serde(default)]
        consecutive_failures: u32,
    },
}
