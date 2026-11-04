//! Private-safe, latest-wins diagnostics for the mandatory Sliding Sync runtime.
//!
//! This lane is deliberately separate from product `AppState` and from the
//! bounded diagnostic event ring. Its public API accepts only fixed enums,
//! booleans and counters, so Matrix identifiers, URLs, tokens, positions and
//! raw SDK errors cannot enter the copied report.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::direct_message_classification::DirectAccountDataSource;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncDiscoveryState {
    #[default]
    NotStarted,
    Probing,
    Supported,
    Unsupported,
    Unreachable,
    InvalidResponse,
}

impl SlidingSyncDiscoveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Probing => "probing",
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Unreachable => "unreachable",
            Self::InvalidResponse => "invalid_response",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncDiscoverySource {
    #[default]
    Unknown,
    Versions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncHttpStatusClass {
    #[default]
    Unknown,
    Success,
    ClientError,
    ServerError,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum SlidingSyncRequestSchema {
    #[default]
    #[serde(rename = "element_x_all_rooms")]
    ElementXAllRooms,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum SlidingSyncEngine {
    #[default]
    #[serde(rename = "SyncService")]
    SyncService,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncSdkVersion {
    #[default]
    Unknown,
    None,
    Native,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncProvisionalHandoffBucket {
    #[default]
    Never,
    Under100Milliseconds,
    UnderOneSecond,
    OneSecondOrMore,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncLifecycle {
    #[default]
    Stopped,
    Starting,
    Running,
    Reconnecting,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncFailureOrigin {
    #[default]
    None,
    RoomList,
    Encryption,
    Supervisor,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncFailureKind {
    #[default]
    None,
    #[serde(rename = "sync_failed_http")]
    Http,
    #[serde(rename = "sync_failed_auth")]
    Auth,
    #[serde(rename = "sync_failed_store")]
    Store,
    #[serde(rename = "sync_failed_protocol")]
    Protocol,
    #[serde(rename = "sync_failed_internal")]
    Internal,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncFailureStage {
    #[default]
    None,
    RoomListSlidingSync,
    RoomListEventCache,
    RoomListProjection,
    EncryptionSlidingSync,
    EncryptionLock,
    EncryptionClient,
    Supervisor,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncHttpErrorSource {
    #[default]
    None,
    Transport,
    ServerResponse,
    ResponseDecode,
    RequestBuild,
    TokenRefresh,
    Cached,
    Tls,
    NotHttp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncHttpStatus {
    #[default]
    None,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    RateLimited,
    ClientError,
    ServerError,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncMatrixErrorKind {
    #[default]
    None,
    Unknown,
    BadJson,
    InvalidParam,
    MissingParam,
    NotJson,
    NotFound,
    Unauthorized,
    MissingToken,
    UnknownToken,
    Forbidden,
    UnknownPos,
    Unrecognized,
    LimitExceeded,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlidingSyncFailureRetryability {
    #[default]
    None,
    Transient,
    Permanent,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SlidingSyncFailureDiagnostic {
    pub origin: SlidingSyncFailureOrigin,
    pub kind: SlidingSyncFailureKind,
    pub stage: SlidingSyncFailureStage,
    pub http_error_source: SlidingSyncHttpErrorSource,
    pub http_status: SlidingSyncHttpStatus,
    pub matrix_error_kind: SlidingSyncMatrixErrorKind,
    pub retryability: SlidingSyncFailureRetryability,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum DiagnosticAgeBucket {
    #[default]
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "<1m")]
    UnderOneMinute,
    #[serde(rename = "1-5m")]
    OneToFiveMinutes,
    #[serde(rename = "5-30m")]
    FiveToThirtyMinutes,
    #[serde(rename = "30m-2h")]
    ThirtyMinutesToTwoHours,
    #[serde(rename = ">=2h")]
    TwoHoursOrMore,
}

fn age_bucket(observed_at: Option<Instant>) -> DiagnosticAgeBucket {
    let Some(elapsed) = observed_at.map(|instant| instant.elapsed()) else {
        return DiagnosticAgeBucket::Never;
    };
    if elapsed < Duration::from_secs(60) {
        DiagnosticAgeBucket::UnderOneMinute
    } else if elapsed < Duration::from_secs(5 * 60) {
        DiagnosticAgeBucket::OneToFiveMinutes
    } else if elapsed < Duration::from_secs(30 * 60) {
        DiagnosticAgeBucket::FiveToThirtyMinutes
    } else if elapsed < Duration::from_secs(2 * 60 * 60) {
        DiagnosticAgeBucket::ThirtyMinutesToTwoHours
    } else {
        DiagnosticAgeBucket::TwoHoursOrMore
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlidingSyncDiscoveryDiagnostic {
    state: SlidingSyncDiscoveryState,
    advertised: bool,
    source: SlidingSyncDiscoverySource,
    http_status_class: SlidingSyncHttpStatusClass,
}

impl SlidingSyncDiscoveryDiagnostic {
    pub fn supported() -> Self {
        Self {
            state: SlidingSyncDiscoveryState::Supported,
            advertised: true,
            source: SlidingSyncDiscoverySource::Versions,
            http_status_class: SlidingSyncHttpStatusClass::Success,
        }
    }

    pub fn from_result(result: &koushi_sdk::SlidingSyncDiscoveryResult) -> Self {
        use koushi_sdk::{DiscoverySource, HttpStatusClass, SlidingSyncDiscoveryResult};

        let map_status = |status: Option<HttpStatusClass>| match status {
            Some(HttpStatusClass::Success) => SlidingSyncHttpStatusClass::Success,
            Some(HttpStatusClass::ClientError) => SlidingSyncHttpStatusClass::ClientError,
            Some(HttpStatusClass::ServerError) => SlidingSyncHttpStatusClass::ServerError,
            Some(HttpStatusClass::Other) => SlidingSyncHttpStatusClass::Other,
            None => SlidingSyncHttpStatusClass::Unknown,
        };
        match result {
            SlidingSyncDiscoveryResult::Supported {
                source,
                advertised,
                http_status_class,
            } => Self {
                state: SlidingSyncDiscoveryState::Supported,
                advertised: *advertised,
                source: match source {
                    DiscoverySource::Versions => SlidingSyncDiscoverySource::Versions,
                },
                http_status_class: map_status(*http_status_class),
            },
            SlidingSyncDiscoveryResult::Unsupported {
                advertised,
                http_status_class,
            } => Self {
                state: SlidingSyncDiscoveryState::Unsupported,
                advertised: *advertised,
                source: SlidingSyncDiscoverySource::Versions,
                http_status_class: map_status(*http_status_class),
            },
            SlidingSyncDiscoveryResult::Unreachable { .. } => Self {
                state: SlidingSyncDiscoveryState::Unreachable,
                advertised: false,
                source: SlidingSyncDiscoverySource::Versions,
                http_status_class: SlidingSyncHttpStatusClass::Unknown,
            },
            SlidingSyncDiscoveryResult::InvalidResponse {
                http_status_class, ..
            } => Self {
                state: SlidingSyncDiscoveryState::InvalidResponse,
                advertised: false,
                source: SlidingSyncDiscoverySource::Versions,
                http_status_class: map_status(*http_status_class),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlidingSyncDiagnosticsSnapshot {
    pub discovery_state: SlidingSyncDiscoveryState,
    pub advertised: bool,
    pub discovery_source: SlidingSyncDiscoverySource,
    pub last_probe_age_bucket: DiagnosticAgeBucket,
    pub last_http_status_class: SlidingSyncHttpStatusClass,
    pub request_schema: SlidingSyncRequestSchema,
    pub engine: SlidingSyncEngine,
    pub sdk_sliding_sync_version: SlidingSyncSdkVersion,
    pub room_list_share_pos: bool,
    pub encryption_share_pos: bool,
    pub encryption_connection_profile: &'static str,
    pub encryption_extension_profile: &'static str,
    pub provisional_encryption_started: bool,
    pub provisional_first_response_seen: bool,
    pub provisional_stopped_before_first_response: bool,
    pub provisional_to_normal_handoff_bucket: SlidingSyncProvisionalHandoffBucket,
    pub lifecycle: SlidingSyncLifecycle,
    pub connectivity_proven: bool,
    pub committed_generation: u64,
    pub last_success_age_bucket: DiagnosticAgeBucket,
    pub consecutive_failure_count: u64,
    pub last_failure_origin: SlidingSyncFailureOrigin,
    pub last_failure_kind: SlidingSyncFailureKind,
    pub last_failure_stage: SlidingSyncFailureStage,
    pub last_http_error_source: SlidingSyncHttpErrorSource,
    pub last_http_status: SlidingSyncHttpStatus,
    pub last_matrix_error_kind: SlidingSyncMatrixErrorKind,
    pub last_failure_retryability: SlidingSyncFailureRetryability,
    pub room_list_task_running: bool,
    pub encryption_task_running: bool,
    pub pos_present: bool,
    pub direct_account_data_source: DirectAccountDataSource,
    pub direct_mapped_room_count: u64,
    pub direct_target_count: u64,
    pub projected_dm_count: u64,
    pub explicit_dm_count: u64,
    pub fallback_dm_count: u64,
    pub direct_non_dm_count: u64,
    pub direct_invalid_entry_count: u64,
    pub direct_event_wake_count: u64,
    pub direct_event_applied_count: u64,
    pub direct_event_stream_running: bool,
}

impl Default for SlidingSyncDiagnosticsSnapshot {
    fn default() -> Self {
        Self {
            discovery_state: SlidingSyncDiscoveryState::NotStarted,
            advertised: false,
            discovery_source: SlidingSyncDiscoverySource::Unknown,
            last_probe_age_bucket: DiagnosticAgeBucket::Never,
            last_http_status_class: SlidingSyncHttpStatusClass::Unknown,
            request_schema: SlidingSyncRequestSchema::ElementXAllRooms,
            engine: SlidingSyncEngine::SyncService,
            sdk_sliding_sync_version: SlidingSyncSdkVersion::Unknown,
            room_list_share_pos: true,
            encryption_share_pos: false,
            encryption_connection_profile: "sdk_default_encryption",
            encryption_extension_profile: "e2ee_to_device",
            provisional_encryption_started: false,
            provisional_first_response_seen: false,
            provisional_stopped_before_first_response: false,
            provisional_to_normal_handoff_bucket: SlidingSyncProvisionalHandoffBucket::Never,
            lifecycle: SlidingSyncLifecycle::Stopped,
            connectivity_proven: false,
            committed_generation: 0,
            last_success_age_bucket: DiagnosticAgeBucket::Never,
            consecutive_failure_count: 0,
            last_failure_origin: SlidingSyncFailureOrigin::None,
            last_failure_kind: SlidingSyncFailureKind::None,
            last_failure_stage: SlidingSyncFailureStage::None,
            last_http_error_source: SlidingSyncHttpErrorSource::None,
            last_http_status: SlidingSyncHttpStatus::None,
            last_matrix_error_kind: SlidingSyncMatrixErrorKind::None,
            last_failure_retryability: SlidingSyncFailureRetryability::None,
            room_list_task_running: false,
            encryption_task_running: false,
            pos_present: false,
            direct_account_data_source: DirectAccountDataSource::Unavailable,
            direct_mapped_room_count: 0,
            direct_target_count: 0,
            projected_dm_count: 0,
            explicit_dm_count: 0,
            fallback_dm_count: 0,
            direct_non_dm_count: 0,
            direct_invalid_entry_count: 0,
            direct_event_wake_count: 0,
            direct_event_applied_count: 0,
            direct_event_stream_running: false,
        }
    }
}

#[derive(Debug, Default)]
struct SlidingSyncDiagnosticsState {
    snapshot: SlidingSyncDiagnosticsSnapshot,
    last_probe_at: Option<Instant>,
    last_success_at: Option<Instant>,
    provisional_stopped_at: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct SlidingSyncDiagnostics {
    inner: Arc<Mutex<SlidingSyncDiagnosticsState>>,
    owner: SyncDiagnosticsOwner,
}

/// Process-unique identity of the account runtime that owns one sliding-sync
/// diagnostics lane.
///
/// `run_generation` already names *which run* of a sync owner is in flight, but
/// it restarts at 1 for every account runtime, so it cannot separate the runs
/// of several runtimes that share one process (the headless aggregate lane runs
/// one `CoreRuntime` per participant). Records therefore carry both: this owner
/// says which runtime produced them, `backend_generation` says which run of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SyncDiagnosticsOwner(u64);

impl SyncDiagnosticsOwner {
    fn allocate() -> Self {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_OWNER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Default for SlidingSyncDiagnostics {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SlidingSyncDiagnosticsState::default())),
            owner: SyncDiagnosticsOwner::allocate(),
        }
    }
}

impl SlidingSyncDiagnostics {
    /// The process-unique owner stamped on this runtime's diagnostic records.
    pub fn owner(&self) -> SyncDiagnosticsOwner {
        self.owner
    }

    pub fn snapshot(&self) -> SlidingSyncDiagnosticsSnapshot {
        let state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut snapshot = state.snapshot.clone();
        snapshot.last_probe_age_bucket = age_bucket(state.last_probe_at);
        snapshot.last_success_age_bucket = age_bucket(state.last_success_at);
        snapshot
    }

    pub fn discovery_started(&self) {
        self.update(|state| state.snapshot.discovery_state = SlidingSyncDiscoveryState::Probing);
    }

    pub fn admission_discovery_started(&self) {
        self.update(|state| {
            *state = SlidingSyncDiagnosticsState::default();
            state.snapshot.discovery_state = SlidingSyncDiscoveryState::Probing;
        });
    }

    pub fn record_discovery(&self, result: SlidingSyncDiscoveryDiagnostic) {
        self.update(|state| {
            state.snapshot.discovery_state = result.state;
            state.snapshot.advertised = result.advertised;
            state.snapshot.discovery_source = result.source;
            state.snapshot.last_http_status_class = result.http_status_class;
            state.last_probe_at = Some(Instant::now());
        });
    }

    pub fn runtime_profile(&self, version: SlidingSyncSdkVersion) {
        self.update(|state| state.snapshot.sdk_sliding_sync_version = version);
    }

    pub fn direct_classification_initialized(
        &self,
        source: DirectAccountDataSource,
        mapped_rooms: u64,
        targets: u64,
    ) {
        self.update(|state| {
            state.snapshot.direct_account_data_source = source;
            state.snapshot.direct_mapped_room_count = mapped_rooms;
            state.snapshot.direct_target_count = targets;
            state.snapshot.direct_event_stream_running = true;
        });
    }

    pub fn direct_event_recorded(
        &self,
        source: DirectAccountDataSource,
        mapped_rooms: u64,
        targets: u64,
        wakes: u64,
        applied: u64,
        stream_running: bool,
    ) {
        self.update(|state| {
            state.snapshot.direct_account_data_source = source;
            state.snapshot.direct_mapped_room_count = mapped_rooms;
            state.snapshot.direct_target_count = targets;
            state.snapshot.direct_event_wake_count = wakes;
            state.snapshot.direct_event_applied_count = applied;
            state.snapshot.direct_event_stream_running = stream_running;
        });
    }

    pub fn direct_projection_recorded(
        &self,
        projected_dms: u64,
        explicit_dms: u64,
        fallback_dms: u64,
        non_dms: u64,
        invalid_entries: u64,
    ) {
        self.update(|state| {
            state.snapshot.projected_dm_count = projected_dms;
            state.snapshot.explicit_dm_count = explicit_dms;
            state.snapshot.fallback_dm_count = fallback_dms;
            state.snapshot.direct_non_dm_count = non_dms;
            state.snapshot.direct_invalid_entry_count = invalid_entries;
        });
    }

    pub fn provisional_encryption_started(&self) {
        self.update(|state| {
            state.snapshot.provisional_encryption_started = true;
            state.snapshot.provisional_first_response_seen = false;
            state.snapshot.provisional_stopped_before_first_response = false;
            state.provisional_stopped_at = None;
        });
    }

    pub fn provisional_encryption_first_response_seen(&self) {
        self.update(|state| state.snapshot.provisional_first_response_seen = true);
    }

    pub fn provisional_encryption_stopped(&self) {
        self.update(|state| {
            if state.snapshot.provisional_encryption_started {
                state.snapshot.provisional_stopped_before_first_response =
                    !state.snapshot.provisional_first_response_seen;
                state.provisional_stopped_at = Some(Instant::now());
            }
        });
    }

    pub fn sync_started(&self, _generation: u64) {
        self.update(|state| {
            state.snapshot.provisional_to_normal_handoff_bucket = match state
                .provisional_stopped_at
                .map(|instant| instant.elapsed())
            {
                Some(elapsed) if elapsed < Duration::from_millis(100) => {
                    SlidingSyncProvisionalHandoffBucket::Under100Milliseconds
                }
                Some(elapsed) if elapsed < Duration::from_secs(1) => {
                    SlidingSyncProvisionalHandoffBucket::UnderOneSecond
                }
                Some(_) => SlidingSyncProvisionalHandoffBucket::OneSecondOrMore,
                None => SlidingSyncProvisionalHandoffBucket::Never,
            };
            state.snapshot.lifecycle = SlidingSyncLifecycle::Starting;
            state.snapshot.room_list_task_running = true;
            state.snapshot.encryption_task_running = true;
        });
    }

    pub fn sync_offline(&self, failure: SlidingSyncFailureDiagnostic) {
        self.update(|state| {
            state.snapshot.lifecycle = SlidingSyncLifecycle::Reconnecting;
            state.snapshot.consecutive_failure_count =
                state.snapshot.consecutive_failure_count.saturating_add(1);
            state.snapshot.last_failure_origin = failure.origin;
            state.snapshot.last_failure_kind = failure.kind;
            state.snapshot.last_failure_stage = failure.stage;
            state.snapshot.last_http_error_source = failure.http_error_source;
            state.snapshot.last_http_status = failure.http_status;
            state.snapshot.last_matrix_error_kind = failure.matrix_error_kind;
            state.snapshot.last_failure_retryability = failure.retryability;
            state.snapshot.room_list_task_running = false;
            state.snapshot.encryption_task_running = false;
        });
    }

    pub fn response_committed(&self, generation: u64, pos_present: bool) {
        self.update(|state| {
            state.snapshot.lifecycle = SlidingSyncLifecycle::Running;
            state.snapshot.connectivity_proven = true;
            state.snapshot.committed_generation = generation;
            state.snapshot.consecutive_failure_count = 0;
            state.snapshot.room_list_task_running = true;
            state.snapshot.encryption_task_running = true;
            state.snapshot.pos_present = pos_present;
            state.last_success_at = Some(Instant::now());
        });
    }

    pub fn failed(&self, failure: SlidingSyncFailureDiagnostic) {
        self.update(|state| {
            state.snapshot.lifecycle = SlidingSyncLifecycle::Failed;
            state.snapshot.consecutive_failure_count =
                state.snapshot.consecutive_failure_count.saturating_add(1);
            state.snapshot.last_failure_origin = failure.origin;
            state.snapshot.last_failure_kind = failure.kind;
            state.snapshot.last_failure_stage = failure.stage;
            state.snapshot.last_http_error_source = failure.http_error_source;
            state.snapshot.last_http_status = failure.http_status;
            state.snapshot.last_matrix_error_kind = failure.matrix_error_kind;
            state.snapshot.last_failure_retryability = failure.retryability;
            state.snapshot.room_list_task_running = false;
            state.snapshot.encryption_task_running = false;
        });
    }

    pub fn stopped(&self) {
        self.update(|state| {
            state.snapshot.lifecycle = SlidingSyncLifecycle::Stopped;
            state.snapshot.room_list_task_running = false;
            state.snapshot.encryption_task_running = false;
        });
    }

    fn update(&self, update: impl FnOnce(&mut SlidingSyncDiagnosticsState)) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state);
    }
}
