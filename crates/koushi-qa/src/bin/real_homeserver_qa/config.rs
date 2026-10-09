use super::{CreateRoomOptions, CreateRoomVisibility};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Env var constants
// ---------------------------------------------------------------------------

pub(super) const ENV_DATA_DIR: &str = "KOUSHI_QA_DATA_DIR";
pub(super) const ENV_REAL_QA_SCENARIO: &str = "KOUSHI_REAL_QA_SCENARIO";

#[cfg(any(debug_assertions, test))]
pub(super) const ENV_CREDENTIALS_PATH: &str = "KOUSHI_REAL_QA_CREDENTIALS_PATH";
#[cfg(any(debug_assertions, test))]
pub(super) const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";
/// When set to "1", the `startup_latency` scenario logs out at teardown so the
/// QA device is removed from the homeserver. Unset by default: the session is
/// kept so run 2+ can restore rather than login.
#[cfg(any(debug_assertions, test))]
pub(super) const ENV_STARTUP_LAT_TEARDOWN: &str = "KOUSHI_STARTUP_LAT_TEARDOWN";
/// Number of backward paginate pages to issue in the `startup_latency` scenario.
#[cfg(any(debug_assertions, test))]
pub(super) const STARTUP_LAT_PAGES: usize = 3;

#[cfg(any(debug_assertions, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RealQaScenario {
    Compat,
    SpaceCompat,
    All,
    /// Read-only timing probe: restore-or-login, sync to ready, subscribe and
    /// paginate a target room, emit `startup_lat phase=… ms=…` tokens only.
    StartupLatency,
}

#[cfg(any(debug_assertions, test))]
impl RealQaScenario {
    pub(super) fn from_env() -> Result<Self, String> {
        Self::from_env_value(std::env::var(ENV_REAL_QA_SCENARIO).ok())
    }

    pub(super) fn from_env_value(value: Option<String>) -> Result<Self, String> {
        match value.as_deref() {
            None | Some("space_compat") => Ok(Self::SpaceCompat),
            Some("compat") => Ok(Self::Compat),
            Some("all") => Ok(Self::All),
            Some("startup_latency") => Ok(Self::StartupLatency),
            Some(other) => Err(format!(
                "unsupported {ENV_REAL_QA_SCENARIO} value '{other}'; \
                 expected compat, space_compat, all, or startup_latency"
            )),
        }
    }

    pub(super) fn includes_space_stage(self) -> bool {
        matches!(self, Self::SpaceCompat | Self::All)
    }
}

// ---------------------------------------------------------------------------
// Timeout constants - matrix.org is slower than local servers
// ---------------------------------------------------------------------------

/// Standard per-event wait.
pub(super) const EVENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Extended timeout for sync operations (matrix.org initial sync can be slow).
pub(super) const SYNC_TIMEOUT: Duration = Duration::from_secs(120);
/// Extended timeout for room list non-empty wait.
pub(super) const ROOM_LIST_TIMEOUT: Duration = Duration::from_secs(120);
/// Shorter timeout for the optional space-child projection observation.
pub(super) const SPACE_CHILD_PROJECTION_TIMEOUT: Duration = Duration::from_secs(20);
/// Timeout for search indexing (ngram index updated by sync loop).
pub(super) const SEARCH_TIMEOUT: Duration = Duration::from_secs(90);
/// Timeout for edit/redact confirmation (sync round-trip).
pub(super) const EDIT_REDACT_TIMEOUT: Duration = Duration::from_secs(90);
/// Timeout for paginate-to-EndReached.
pub(super) const PAGINATE_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) fn private_room_options(name: impl Into<String>, encrypted: bool) -> CreateRoomOptions {
    CreateRoomOptions {
        name: name.into(),
        topic: None,
        alias_localpart: None,
        encrypted,
        invited_only: false,
        visibility: CreateRoomVisibility::Private,
        parent_space: None,
        access_policy: None,
        history: None,
    }
}

// ---------------------------------------------------------------------------
// Credentials (only loaded in debug/test builds)
// ---------------------------------------------------------------------------
#[cfg(any(debug_assertions, test))]
pub(super) struct RealHomeserverQaMessagePlan {
    pub(super) search_token: String,
    pub(super) msg1_body: String,
    pub(super) search_probe_body: String,
    pub(super) msg2_body: String,
    pub(super) edited_body: String,
    pub(super) reply_body: String,
}

#[cfg(any(debug_assertions, test))]
pub(super) fn build_real_homeserver_qa_message_plan(ts: u64) -> RealHomeserverQaMessagePlan {
    let search_token = format!("real-qa-search-{}-{}", std::process::id(), ts);
    RealHomeserverQaMessagePlan {
        search_token: search_token.clone(),
        msg1_body: "Real homeserver QA message 1".to_owned(),
        search_probe_body: format!("Real homeserver QA search probe {search_token}"),
        msg2_body: "Real homeserver QA message 2".to_owned(),
        edited_body: "Real homeserver QA message 1 EDITED".to_owned(),
        reply_body: "Real homeserver QA reply to message 1".to_owned(),
    }
}

#[cfg(any(debug_assertions, test))]
pub(super) fn real_qa_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var(ENV_DATA_DIR) {
        return std::path::PathBuf::from(dir);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    std::env::temp_dir()
        .join("koushi-desktop-real-qa")
        .join(format!("{}_{}", std::process::id(), ts))
}
