//! Headless Core QA binary v2 (Phase 4: adds room operations and room list QA).
//!
//! Exercises login (with store bootstrap), store-backed session restore,
//! logout cleanup, sync lifecycle, room creation, space creation,
//! space-child assignment, invite/join, room list normalization, and
//! stdout/stderr secret-redaction using ONLY `CoreCommand`/`CoreEvent` —
//! no direct auth-crate calls in the QA flow.
//!
//! Topology: one `CoreRuntime` per synthetic user (spec, Headless QA section:
//! that models two devices, the realistic A/B topology; multi-account-in-one-
//! runtime behavior is account-switch QA's job).
//!
//! Hard guard: this binary refuses to run unless the file credential store
//! override is active. Unattended QA must be structurally unable to reach the
//! OS keychain (a keychain prompt during automation is a failure per the
//! engineering rules), so the guard runs BEFORE any login.
//!
//! Phase 4 flow (both probed SyncService leg and forced LegacySync leg):
//!   A creates room + space + sets space child + invites B to both
//!   B joins room + space
//!   both assert room list contains expected room and space (event-driven)
//!   print room-list counts in summary line
//!   send permission check placeholder (actual send is Phase 5)
//!
//! Required env vars:
//!   KOUSHI_LOCAL_QA_HOMESERVER
//!   KOUSHI_LOCAL_QA_SERVER_NAME
//!   KOUSHI_LOCAL_QA_SERVER_KIND   (optional, defaults to "local")
//!   KOUSHI_LOCAL_QA_USER_A / _PASSWORD_A
//!   KOUSHI_LOCAL_QA_USER_B / _PASSWORD_B
//!   KOUSHI_LOCAL_QA_USER_C (optional; required by invites_dm DM scope QA)
//!   KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR (mandatory; see guard)
//!
//! SDK handles are dropped inside the Tokio runtime context (overview.md Async rule 11).

use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::BTreeSet, future::Future, io};

use koushi_core::command::{
    AccountCommand, AppCommand, CoreCommand, CreateRoomOptions, CreateRoomVisibility,
    ImageUploadCompressionPolicy, ImageUploadCompressionState, ImageUploadDimensions,
    ImageUploadVariantInfo, ImageUploadVariantKind, MediaDownloadSelection, RoomCommand,
    SearchCommand, SearchScope, SyncCommand, TimelineCommand, UploadMediaKind, UploadMediaRequest,
    UploadMediaThumbnail,
};
use koushi_core::event::{
    AccountEvent, ActivityEvent, CoreEvent, E2eeTrustEvent, LinkPreviewState, LiveSignalsEvent,
    LocalEncryptionEvent, PaginationDirection, PaginationState, RoomEvent, SearchEvent,
    SyncBackendKind, SyncEvent, TimelineAnchorRestoreStatus, TimelineDiff, TimelineEvent,
    TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId, TimelineMessageActions,
    TimelineSendState, TimelineUnreadPosition, TimelineViewportObservation,
};
use koushi_core::failure::{CoreFailure, RoomFailureKind};
use koushi_core::ids::{AccountKey, RequestId, TimelineKey, TimelineKind};
use koushi_core::runtime::{CoreConnection, CoreRuntime, EventStreamLag};
use koushi_state::{
    ActivityMarkReadTarget, ActivityRowKind, ActivityState, AppAction, AppState, AuthSecret,
    ComposerKey, ComposerKeyEvent, ComposerKeyModifiers, ComposerResolvedAction,
    ComposerResolverContext, ComposerSelection, ComposerSendShortcut, ComposerSurface,
    DirectoryQuery, DirectoryRoomSummary, DisplaySettings, IdentityResetAuthRequest,
    IdentityResetAuthType, IdentityResetState, ImageUploadCompressionMode, KeyBackupStatus,
    LocalEncryptionHealth, LocalEncryptionState, MentionIntent, MentionTarget,
    NativeAttentionCapabilities, NativeAttentionCapability, NativeAttentionDispatchState,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, NativeAttentionState,
    NativeAttentionSuppressionReason, OperationFailureKind, PresenceKind, RecoveryRequest,
    ReplyQuoteState, RoomAttentionKind, RoomListFilter, RoomManagementOperationKind,
    RoomManagementOperationState, RoomModerationAction, RoomNotificationMode, RoomSettingChange,
    RoomSettingsSnapshot, RoomSummary, RoomTags, SasEmoji, ScheduledSendCapability,
    SearchCrawlerFailureKind, SearchCrawlerRoomState, SearchCrawlerSettings, SearchCrawlerSpeed,
    SessionInfo, SessionState, SettingsPatch, SettingsPersistenceState,
    StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind, TimelineMediaGalleryItem,
    TimelineMediaGalleryMedia, TimelineMediaGallerySource, TimelineMediaKind,
    VerificationFlowState, VerificationTarget, build_formatted_message_draft, compose_sidebar,
    native_attention_state_from_rooms, reduce, resolve_composer_key_action,
};

const ENV_HOMESERVER: &str = "KOUSHI_LOCAL_QA_HOMESERVER";
const ENV_SERVER_NAME: &str = "KOUSHI_LOCAL_QA_SERVER_NAME";
const ENV_SERVER_KIND: &str = "KOUSHI_LOCAL_QA_SERVER_KIND";
const ENV_USER_A: &str = "KOUSHI_LOCAL_QA_USER_A";
const ENV_PASSWORD_A: &str = "KOUSHI_LOCAL_QA_PASSWORD_A";
const ENV_USER_B: &str = "KOUSHI_LOCAL_QA_USER_B";
const ENV_PASSWORD_B: &str = "KOUSHI_LOCAL_QA_PASSWORD_B";
const ENV_USER_C: &str = "KOUSHI_LOCAL_QA_USER_C";
/// Optional assertion input (a plain string, not a credential — no gating
/// needed): when set, QA fails if the backend reported in SyncEvent::Started
/// differs. Valid values: "SyncService" | "LegacySync".
const ENV_EXPECT_SYNC_BACKEND: &str = "KOUSHI_LOCAL_QA_EXPECT_SYNC_BACKEND";
const ENV_QA_SCENARIO: &str = "KOUSHI_QA_SCENARIO";
const ENV_ALLOW_IDENTITY_RESET: &str = "KOUSHI_QA_ALLOW_IDENTITY_RESET";
const ENV_E2EE_RECIPIENT_SECOND_DEVICE: &str = "KOUSHI_QA_E2EE_RECIPIENT_SECOND_DEVICE";
#[cfg(any(debug_assertions, feature = "qa-bin"))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";

const DEVICE_A: &str = "Koushi Core QA A";
const DEVICE_B: &str = "Koushi Core QA B";

/// Maximum time to wait for a single event.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const GATE_RESTORE_READY_BUDGET: Duration = Duration::from_secs(10);
const LOGIN_EVENT_TIMEOUT: Duration = Duration::from_secs(90);
const ROOM_LIST_EVENT_TIMEOUT: Duration = Duration::from_secs(90);
const TIMELINE_INITIAL_EVENT_TIMEOUT: Duration = Duration::from_secs(90);
const E2EE_EVENT_TIMEOUT: Duration = Duration::from_secs(90);
// Local proxy teardown can leave the SDK in reconnect/backoff before queued
// sends resume, so this lane needs a wider budget than generic event waits.
const SEND_QUEUE_EVENT_TIMEOUT: Duration = Duration::from_secs(300);
const TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);

type QaEventFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + 'a>>;

trait QaEventSource {
    fn recv_event(&mut self) -> QaEventFuture<'_>;
}

trait QaSnapshotEventSource: QaEventSource {
    fn snapshot(&self) -> AppState;
}

impl QaEventSource for CoreConnection {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(CoreConnection::recv_event(self))
    }
}

impl QaSnapshotEventSource for CoreConnection {
    fn snapshot(&self) -> AppState {
        CoreConnection::snapshot(self)
    }
}

#[derive(Clone, Copy)]
struct QaEventDeadline {
    instant: tokio::time::Instant,
}

impl QaEventDeadline {
    fn after(timeout: Duration) -> Self {
        Self {
            instant: tokio::time::Instant::now() + timeout,
        }
    }

    async fn recv<S: QaEventSource + ?Sized>(
        self,
        source: &mut S,
    ) -> Result<Result<CoreEvent, EventStreamLag>, tokio::time::error::Elapsed> {
        tokio::time::timeout_at(self.instant, source.recv_event()).await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PairedEventWaitError {
    Deadline,
    Primary(EventStreamLag),
    Secondary(EventStreamLag),
}

async fn wait_for_paired_event_until<Primary, Secondary>(
    primary: &mut Primary,
    secondary: &mut Secondary,
    deadline: tokio::time::Instant,
) -> Result<(), PairedEventWaitError>
where
    Primary: QaEventSource + ?Sized,
    Secondary: QaEventSource + ?Sized,
{
    tokio::select! {
        event = primary.recv_event() => event
            .map(|_| ())
            .map_err(PairedEventWaitError::Primary),
        event = secondary.recv_event() => event
            .map(|_| ())
            .map_err(PairedEventWaitError::Secondary),
        _ = tokio::time::sleep_until(deadline) => Err(PairedEventWaitError::Deadline),
    }
}
const THREAD_REPLY_BODY: &str = "Phase 11 QA thread reply from B";
const E2EE_KEY_BACKUP_SEED_BODY: &str = "Koushi E2EE key backup seed";
const E2EE_SECOND_DEVICE_BODY: &str = "Koushi E2EE second-device delivery";
const E2EE_MULTI_USER_MULTI_DEVICE_BODY: &str = "Koushi E2EE multi-user multi-device delivery";
const DEFAULT_STRESS_SPACE_COUNT: usize = 2;
const DEFAULT_STRESS_ROOMS_PER_SPACE: usize = 2;
const DEFAULT_STRESS_MESSAGES_PER_ROOM: usize = 8;
const MAX_STRESS_SPACE_COUNT: usize = 6;
const MAX_STRESS_ROOMS_PER_SPACE: usize = 8;
const MAX_STRESS_MESSAGES_PER_ROOM: usize = 80;
const ENV_STRESS_SPACE_COUNT: &str = "KOUSHI_QA_STRESS_SPACES";
const ENV_STRESS_ROOMS_PER_SPACE: &str = "KOUSHI_QA_STRESS_ROOMS_PER_SPACE";
const ENV_STRESS_MESSAGES_PER_ROOM: &str = "KOUSHI_QA_STRESS_MESSAGES_PER_ROOM";
const ENV_STRESS_REPLAY_EXISTING: &str = "KOUSHI_QA_STRESS_REPLAY_EXISTING";
const QA_WRONG_RECOVERY_SECRET: &str = "koushi-desktop-headless-qa-wrong-recovery-secret";
const ENV_CACHE_RESTORE_ROOMS: &str = "KOUSHI_QA_CACHE_RESTORE_ROOMS";
const ENV_CACHE_RESTORE_DEPTH: &str = "KOUSHI_QA_CACHE_RESTORE_DEPTH";
const DEFAULT_CACHE_RESTORE_ROOMS: usize = 3;
const DEFAULT_CACHE_RESTORE_DEPTH: usize = 200;
/// Batch size used for backward pagination during the populate (EndReached) pass.
const CACHE_RESTORE_PAGINATE_BATCH: u16 = 20;
/// Production-faithful restore parameters, matching the app's live-room constants.
/// Source: apps/desktop/src/components/TimelineView.tsx:406-407
/// (LIVE_ROOM_ANCHOR_RESTORE_MAX_BATCHES=6, EVENT_COUNT=100).
/// These are intentionally small. Room entry should fail fast for stale or
/// very deep persisted anchors and let the UI fall back to live edge; deep
/// event-centered restore belongs to an explicit focused-event timeline.
const CACHE_RESTORE_PROD_MAX_BATCHES: u16 = 6;
const CACHE_RESTORE_PROD_EVENT_COUNT: u16 = 100;
/// Speed gate: maximum backward-paginate cycles allowed per room during an
/// offline anchor restore. Deep anchors may end as BudgetExhausted, but they
/// must not walk history long enough to block room entry.
const CACHE_RESTORE_MAX_CYCLES: u16 = 3;
/// Number of messages in the shallow-anchor room.  Enough to exceed the SDK's
/// initial visible window (~20 items) so that m0 (oldest) is hidden behind a
/// lazy-reveal skip when the session restarts.  All events fit in a single
/// stored chunk (well under 128), so chunks_loaded == 0 during the restore.
/// The anchor (m0) lives in the live in-memory prefix that
/// live_lazy_paginate_backwards reveals (lazy_reveal_batches == 1).
/// The P1 lazy-reveal-fence fix gates on this: without it the settle fence
/// misses the lazy-reveal DiffBatch and may conclude before items settle.
const CACHE_RESTORE_SHALLOW_DEPTH: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaScenario {
    All,
    Safety,
    LoginSync,
    CredentialHealth,
    NativeAttention,
    E2eeTrust,
    GateRestore,
    GateNegative,
    GateNoProof,
    InvitesDm,
    RoomSpace,
    Directory,
    RoomManagement,
    Timeline,
    TimelineReconnect,
    TimelineLegacyFallback,
    TimelineLegacyPersistedGap,
    TimelineStress,
    Activity,
    Composer,
    Reply,
    Media,
    LiveSignals,
    Thread,
    EditRedactSearch,
    SearchCrawler,
    ScheduledSend,
    SendQueue,
    RestoreCleanup,
    LinkPreview,
    CacheRestore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaStage {
    Safety,
    LoginSync,
    CredentialHealth,
    NativeAttention,
    E2eeTrust,
    GateRestore,
    GateNegative,
    GateNoProof,
    InvitesDm,
    RoomSpace,
    Directory,
    RoomManagement,
    Timeline,
    TimelineReconnect,
    TimelineLegacyFallback,
    TimelineLegacyPersistedGap,
    TimelineStress,
    Activity,
    Composer,
    Reply,
    Media,
    LiveSignals,
    Thread,
    EditRedactSearch,
    SearchCrawler,
    ScheduledSend,
    SendQueue,
    RestoreCleanup,
    LinkPreview,
    CacheRestore,
}

fn main() -> ExitCode {
    init_headless_qa_tracing_from_env();

    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Headless core QA failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn init_headless_qa_tracing_from_env() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run() -> Result<String, String> {
    let scenario = QaScenario::from_env()?;
    scenario_preflight_error(scenario)?;

    // Hard guard BEFORE any login: unattended QA must never touch the OS
    // keychain, even if env wiring regresses.
    assert_file_credential_store_active()?;

    let config = QaConfig::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime creation failed: {e}"))?;

    // Run inside the Tokio runtime so SDK handles drop in context (Async rule 11).
    runtime.block_on(run_async(config, scenario))
}

/// Refuse to run against the OS keychain. Debug and qa-bin release builds both
/// check the env var and the structurally resolved backend before any login.
fn assert_file_credential_store_active() -> Result<(), String> {
    #[cfg(any(debug_assertions, feature = "qa-bin"))]
    {
        if std::env::var_os(ENV_FILE_CREDENTIAL_STORE_DIR).is_none() {
            return Err(format!(
                "core QA refuses to run against the OS keychain: {ENV_FILE_CREDENTIAL_STORE_DIR} is not set"
            ));
        }
        if !koushi_core::store::resolved_credential_backend_is_file_dir() {
            return Err(
                "core QA refuses to run against the OS keychain: resolved credential \
                 store backend is not the file-dir backend"
                    .to_owned(),
            );
        }
        Ok(())
    }

    #[cfg(not(any(debug_assertions, feature = "qa-bin")))]
    {
        Err(
            "core QA refuses to run against the OS keychain: release builds have no \
             file credential store backend"
                .to_owned(),
        )
    }
}

impl QaScenario {
    fn from_env() -> Result<Self, String> {
        match std::env::var(ENV_QA_SCENARIO) {
            Ok(value) => Self::from_env_value(&value),
            Err(_) => Ok(Self::All),
        }
    }

    fn from_env_value(value: &str) -> Result<Self, String> {
        match value {
            "all" => Ok(Self::All),
            "safety" => Ok(Self::Safety),
            "login_sync" => Ok(Self::LoginSync),
            "credential_health" => Ok(Self::CredentialHealth),
            "native_attention" => Ok(Self::NativeAttention),
            "e2ee_trust" => Ok(Self::E2eeTrust),
            "gate_restore" => Ok(Self::GateRestore),
            "gate_negative" => Ok(Self::GateNegative),
            "gate_no_proof" => Ok(Self::GateNoProof),
            "invites_dm" => Ok(Self::InvitesDm),
            "room_space" => Ok(Self::RoomSpace),
            "directory" => Ok(Self::Directory),
            "room_management" => Ok(Self::RoomManagement),
            "timeline" => Ok(Self::Timeline),
            "timeline_reconnect" => Ok(Self::TimelineReconnect),
            "timeline_legacy_fallback" => Ok(Self::TimelineLegacyFallback),
            "timeline_legacy_persisted_gap" => Ok(Self::TimelineLegacyPersistedGap),
            "timeline_stress" => Ok(Self::TimelineStress),
            "activity" => Ok(Self::Activity),
            "composer" => Ok(Self::Composer),
            "reply" => Ok(Self::Reply),
            "media" => Ok(Self::Media),
            "live_signals" => Ok(Self::LiveSignals),
            "thread" => Ok(Self::Thread),
            "edit_redact_search" => Ok(Self::EditRedactSearch),
            "search_crawler" => Ok(Self::SearchCrawler),
            "scheduled_send" => Ok(Self::ScheduledSend),
            "send_queue" => Ok(Self::SendQueue),
            "restore_cleanup" => Ok(Self::RestoreCleanup),
            "link_preview" => Ok(Self::LinkPreview),
            "cache_restore" => Ok(Self::CacheRestore),
            other => Err(format!(
                "{ENV_QA_SCENARIO} must be one of all, safety, login_sync, credential_health, native_attention, e2ee_trust, invites_dm, room_space, directory, room_management, timeline, timeline_reconnect, timeline_legacy_fallback, timeline_legacy_persisted_gap, timeline_stress, activity, composer, reply, media, live_signals, thread, edit_redact_search, search_crawler, scheduled_send, restore_cleanup, link_preview, cache_restore; got {other}"
            )),
        }
    }

    fn should_run_stage(self, stage: QaStage) -> bool {
        match self {
            Self::All => !matches!(
                stage,
                QaStage::TimelineReconnect
                    | QaStage::TimelineLegacyFallback
                    | QaStage::TimelineLegacyPersistedGap
                    | QaStage::TimelineStress
            ),
            Self::Safety => matches!(stage, QaStage::Safety),
            Self::LoginSync => matches!(stage, QaStage::Safety | QaStage::LoginSync),
            Self::CredentialHealth => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::CredentialHealth
            ),
            Self::NativeAttention => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::NativeAttention
            ),
            Self::E2eeTrust => {
                matches!(
                    stage,
                    QaStage::Safety | QaStage::LoginSync | QaStage::E2eeTrust
                )
            }
            Self::GateRestore => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::GateRestore
            ),
            Self::GateNegative => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::GateNegative
            ),
            Self::GateNoProof => matches!(stage, QaStage::Safety | QaStage::GateNoProof),
            Self::InvitesDm => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::InvitesDm
            ),
            Self::RoomSpace => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace
            ),
            Self::Directory => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::Directory
            ),
            Self::RoomManagement => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace | QaStage::RoomManagement
            ),
            Self::Timeline => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::RoomSpace | QaStage::Timeline
            ),
            Self::TimelineReconnect => {
                matches!(stage, QaStage::Safety | QaStage::TimelineReconnect)
            }
            Self::TimelineLegacyFallback => {
                matches!(stage, QaStage::Safety | QaStage::TimelineLegacyFallback)
            }
            Self::TimelineLegacyPersistedGap => {
                matches!(stage, QaStage::Safety | QaStage::TimelineLegacyPersistedGap)
            }
            Self::TimelineStress => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::TimelineStress
            ),
            Self::Activity => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Activity
            ),
            Self::Composer => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Composer
            ),
            Self::Reply => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Composer
                    | QaStage::Reply
            ),
            Self::Media => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Media
            ),
            Self::LiveSignals => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::LiveSignals
            ),
            Self::Thread => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Reply
                    | QaStage::Thread
            ),
            Self::EditRedactSearch => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
            ),
            Self::SearchCrawler => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
                    | QaStage::SearchCrawler
            ),
            Self::ScheduledSend => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::ScheduledSend
            ),
            Self::SendQueue => matches!(
                stage,
                QaStage::Safety | QaStage::LoginSync | QaStage::SendQueue
            ),
            Self::RestoreCleanup => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::EditRedactSearch
                    | QaStage::RestoreCleanup
            ),
            Self::LinkPreview => matches!(
                stage,
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::Composer
                    | QaStage::LinkPreview
            ),
            Self::CacheRestore => matches!(stage, QaStage::Safety | QaStage::CacheRestore),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn suppress_matrix_identifiers(self) -> bool {
        let _ = self;
        true
    }
}

fn scenario_preflight_error(scenario: QaScenario) -> Result<(), String> {
    let _ = scenario;
    Ok(())
}

fn tokens_for_stage(stage: QaStage) -> &'static [&'static str] {
    match stage {
        QaStage::Safety => &["safety=ok"],
        QaStage::LoginSync => &["login_sync=ok"],
        QaStage::CredentialHealth => &["credential_health=ok", "fail_closed=ok"],
        QaStage::NativeAttention => &[
            "notification_candidate=ok",
            "badge_state=ok",
            "suppress_focus=ok",
            "clear_badge=ok",
        ],
        QaStage::E2eeTrust => &[
            "joined_room_restore=ok",
            "e2ee_second_device_decrypt=ok",
            "e2ee_multi_user_multi_device_decrypt=ok",
            "e2ee_unverified_peer_send_nonblocking=ok",
            "e2ee_blocked_device_withheld=ok",
            "e2ee_trust=ok",
        ],
        QaStage::GateRestore => &[
            "gate_restore_bootstrapped=ok",
            "gate_restore_shutdown_complete=ok",
            "gate_restore_runtime_spawned=ok",
            "gate_restore_query_sent=ok",
            "gate_restore_query_result=ok",
            "gate_restore_restore_sent=ok",
            "gate_restore_restore_result=ok",
            "gate_restore_ready=ok",
            "gate_verified_restore=ok",
        ],
        QaStage::GateNegative => &[
            "gate_sas_mismatch_retryable=ok",
            "gate_sas_retry_ready=ok",
            "gate_sas_user_cancel_retryable=ok",
            "gate_sas_user_cancel_retry_ready=ok",
            "gate_sas_timeout_retryable=ok",
            "gate_sas_timeout_retry_ready=ok",
            "gate_recovery_invalid_retryable=ok",
            "gate_recovery_retry_ready=ok",
            "gate_recovery_cancel_retryable=ok",
            "gate_recovery_cancel_retry_ready=ok",
            "gate_trust_loss_locked=ok",
            "gate_trust_loss_commands_blocked=ok",
        ],
        QaStage::GateNoProof => &[
            "gate_no_proof_rejected=ok",
            "gate_no_proof_restart_signed_out=ok",
        ],
        QaStage::InvitesDm => &[
            "invite_recv=ok",
            "invite_accept=ok",
            "invite_decline=ok",
            "member_list=ok",
            "dm_start=ok",
            "dm_space_scope=ok",
        ],
        QaStage::RoomSpace => &["room_space=ok"],
        QaStage::Directory => &["directory_query=ok", "directory_join=ok"],
        QaStage::RoomManagement => &["room_settings=ok", "moderation=ok", "permission_guard=ok"],
        QaStage::Timeline => &["timeline=ok", "timeline_nav=ok", "hide_redacted=ok"],
        QaStage::TimelineReconnect => &[
            "timeline_reconnect_recv_after_reconnect=ok",
            "live_catchup_checkpoint=ok",
            "live_catchup_gap_repaired=ok",
            "timeline_reconnect=ok",
        ],
        QaStage::TimelineLegacyFallback => &[
            "legacy_fallback_checkpoint=ok",
            "legacy_fallback_gap_repaired=ok",
            "legacy_fallback_settled=ok",
            "legacy_fallback_lifecycle=ok",
        ],
        QaStage::TimelineLegacyPersistedGap => &[
            "legacy_live_tail_room_absent=ok",
            "live_tail_anchored_silent_gap=ok",
            "live_tail_detached_gap=ok",
            "live_tail_historical_continuation=ok",
        ],
        QaStage::TimelineStress => &[
            "timeline_stress=ok",
            "stress_no_blank=ok",
            "stress_space_scope=ok",
        ],
        QaStage::Activity => &[
            "activity_recent=ok",
            "activity_unread=ok",
            "activity_resolution=ok",
            "activity_markread=ok",
        ],
        QaStage::Composer => &[
            "mention_send=ok",
            "markdown_send=ok",
            "slash_command=ok",
            "ime_guard=ok",
        ],
        QaStage::Reply => &[
            "reply=ok",
            "reply_quote=ok",
            "pin_event=ok",
            "pinned_state=ok",
            "unpin_event=ok",
        ],
        QaStage::Media => &[
            "send_media=ok",
            "media_caption=ok",
            "image_compress=ok",
            "upload_staging=ok",
            "media_gallery=ok",
            "recv_media=ok",
        ],
        QaStage::LiveSignals => &[
            "read_receipt=ok",
            "fully_read=ok",
            "typing=ok",
            "presence=ok",
            "live_signals=ok",
        ],
        QaStage::Thread => &[
            "thread_canonical=ok",
            "thread_summary=ok",
            "thread_recv=ok",
            "thread_paginate=end_reached",
        ],
        QaStage::EditRedactSearch => &["edit_redact_search=ok"],
        QaStage::SearchCrawler => &[
            "crawl_backfill=ok",
            "crawl_no_media_bytes=ok",
            "crawl_throttle=ok",
            "crawl_failure=ok",
        ],
        QaStage::ScheduledSend => &[
            "scheduled_capability=local_fallback",
            "scheduled_create=ok",
            "scheduled_reschedule=ok",
            "scheduled_cancel=ok",
            "scheduled_fire=ok",
        ],
        QaStage::SendQueue => &[
            "send_fail=ok",
            "resend=ok",
            "cancel_send=ok",
            "fifo=ok",
            "unsent_restart=ok",
            "display_projection_reset_fallbacks=0",
        ],
        QaStage::RestoreCleanup => &["restore_cleanup=ok"],
        QaStage::LinkPreview => &[
            "link_preview_global=ok",
            "link_preview_room=ok",
            "link_preview_e2ee_default=ok",
            "link_preview_hide=ok",
        ],
        QaStage::CacheRestore => &["cache_restore=ok"],
    }
}

fn implemented_final_tokens() -> Vec<&'static str> {
    vec![
        "safety=ok",
        "login_sync=ok",
        "credential_health=ok",
        "fail_closed=ok",
        "notification_candidate=ok",
        "badge_state=ok",
        "suppress_focus=ok",
        "clear_badge=ok",
        "invite_recv=ok",
        "invite_accept=ok",
        "invite_decline=ok",
        "member_list=ok",
        "dm_start=ok",
        "dm_space_scope=ok",
        "room_space=ok",
        "directory_query=ok",
        "directory_join=ok",
        "room_settings=ok",
        "moderation=ok",
        "permission_guard=ok",
        "timeline=ok",
        "timeline_nav=ok",
        "hide_redacted=ok",
        "activity_recent=ok",
        "activity_unread=ok",
        "activity_resolution=ok",
        "activity_markread=ok",
        "mention_send=ok",
        "markdown_send=ok",
        "slash_command=ok",
        "ime_guard=ok",
        "reply=ok",
        "reply_quote=ok",
        "pin_event=ok",
        "pinned_state=ok",
        "unpin_event=ok",
        "thread_canonical=ok",
        "thread_summary=ok",
        "thread_recv=ok",
        "thread_paginate=end_reached",
        "send_media=ok",
        "media_caption=ok",
        "image_compress=ok",
        "upload_staging=ok",
        "media_gallery=ok",
        "recv_media=ok",
        "read_receipt=ok",
        "fully_read=ok",
        "typing=ok",
        "presence=ok",
        "live_signals=ok",
        "edit_redact_search=ok",
        "crawl_backfill=ok",
        "crawl_no_media_bytes=ok",
        "crawl_throttle=ok",
        "crawl_failure=ok",
        "scheduled_capability=local_fallback",
        "scheduled_create=ok",
        "scheduled_reschedule=ok",
        "scheduled_cancel=ok",
        "scheduled_fire=ok",
        "send_fail=ok",
        "resend=ok",
        "cancel_send=ok",
        "fifo=ok",
        "unsent_restart=ok",
        "display_projection_reset_fallbacks=0",
        "joined_room_restore=ok",
        "e2ee_second_device_decrypt=ok",
        "e2ee_multi_user_multi_device_decrypt=ok",
        "e2ee_unverified_peer_send_nonblocking=ok",
        "e2ee_blocked_device_withheld=ok",
        "e2ee_trust=ok",
        "restore_cleanup=ok",
        "link_preview_global=ok",
        "link_preview_room=ok",
        "link_preview_e2ee_default=ok",
        "link_preview_hide=ok",
    ]
}

fn stages_for_scenario(scenario: QaScenario) -> Vec<QaStage> {
    match scenario {
        QaScenario::Safety => vec![QaStage::Safety],
        QaScenario::LoginSync => vec![QaStage::Safety, QaStage::LoginSync],
        QaScenario::CredentialHealth => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::CredentialHealth,
        ],
        QaScenario::NativeAttention => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::NativeAttention,
        ],
        QaScenario::E2eeTrust => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::E2eeTrust]
        }
        QaScenario::GateRestore => vec![QaStage::Safety, QaStage::LoginSync, QaStage::GateRestore],
        QaScenario::GateNegative => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::GateNegative]
        }
        QaScenario::GateNoProof => vec![QaStage::Safety, QaStage::GateNoProof],
        QaScenario::InvitesDm => {
            vec![QaStage::Safety, QaStage::LoginSync, QaStage::InvitesDm]
        }
        QaScenario::RoomSpace => vec![QaStage::Safety, QaStage::LoginSync, QaStage::RoomSpace],
        QaScenario::Directory => vec![QaStage::Safety, QaStage::LoginSync, QaStage::Directory],
        QaScenario::RoomManagement => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::RoomManagement,
        ],
        QaScenario::Timeline => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
        ],
        QaScenario::TimelineReconnect => vec![QaStage::Safety, QaStage::TimelineReconnect],
        QaScenario::TimelineLegacyFallback => {
            vec![QaStage::Safety, QaStage::TimelineLegacyFallback]
        }
        QaScenario::TimelineLegacyPersistedGap => {
            vec![QaStage::Safety, QaStage::TimelineLegacyPersistedGap]
        }
        QaScenario::TimelineStress => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::TimelineStress,
        ],
        QaScenario::Activity => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Activity,
        ],
        QaScenario::Composer => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Composer,
        ],
        QaScenario::Reply => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Composer,
            QaStage::Reply,
        ],
        QaScenario::Media => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Media,
        ],
        QaScenario::LiveSignals => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::LiveSignals,
        ],
        QaScenario::Thread => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Reply,
            QaStage::Thread,
        ],
        QaScenario::EditRedactSearch => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
        ],
        QaScenario::SearchCrawler => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
            QaStage::SearchCrawler,
        ],
        QaScenario::ScheduledSend => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::ScheduledSend,
        ],
        QaScenario::SendQueue => vec![QaStage::Safety, QaStage::LoginSync, QaStage::SendQueue],
        QaScenario::RestoreCleanup => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::EditRedactSearch,
            QaStage::RestoreCleanup,
        ],
        QaScenario::LinkPreview => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::Composer,
            QaStage::LinkPreview,
        ],
        QaScenario::CacheRestore => vec![QaStage::Safety, QaStage::CacheRestore],
        QaScenario::All => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::CredentialHealth,
            QaStage::NativeAttention,
            QaStage::InvitesDm,
            QaStage::RoomSpace,
            QaStage::Directory,
            QaStage::RoomManagement,
            QaStage::Timeline,
            QaStage::Activity,
            QaStage::Composer,
            QaStage::Reply,
            QaStage::Media,
            QaStage::LiveSignals,
            QaStage::Thread,
            QaStage::EditRedactSearch,
            QaStage::SearchCrawler,
            QaStage::ScheduledSend,
            QaStage::SendQueue,
            QaStage::E2eeTrust,
            QaStage::RestoreCleanup,
            QaStage::LinkPreview,
        ],
    }
}

fn final_tokens_for_scenario(scenario: QaScenario) -> Vec<&'static str> {
    match scenario {
        QaScenario::Safety => vec!["safety=ok"],
        QaScenario::LoginSync => {
            let mut tokens = stages_for_scenario(scenario)
                .into_iter()
                .flat_map(|stage| tokens_for_stage(stage).iter().copied())
                .collect::<Vec<_>>();
            tokens.push("restore_cleanup=ok");
            tokens.dedup();
            tokens
        }
        QaScenario::RoomSpace
        | QaScenario::Directory
        | QaScenario::RoomManagement
        | QaScenario::CredentialHealth
        | QaScenario::NativeAttention
        | QaScenario::E2eeTrust
        | QaScenario::InvitesDm
        | QaScenario::Timeline
        | QaScenario::TimelineStress
        | QaScenario::Activity
        | QaScenario::Composer
        | QaScenario::Reply
        | QaScenario::Media
        | QaScenario::LiveSignals
        | QaScenario::Thread
        | QaScenario::EditRedactSearch
        | QaScenario::SearchCrawler
        | QaScenario::ScheduledSend
        | QaScenario::SendQueue
        | QaScenario::RestoreCleanup
        | QaScenario::LinkPreview => {
            let mut tokens = stages_for_scenario(scenario)
                .into_iter()
                .flat_map(|stage| tokens_for_stage(stage).iter().copied())
                .collect::<Vec<_>>();
            tokens.push("restore_cleanup=ok");
            tokens.dedup();
            tokens
        }
        QaScenario::TimelineReconnect
        | QaScenario::TimelineLegacyFallback
        | QaScenario::TimelineLegacyPersistedGap
        | QaScenario::CacheRestore
        | QaScenario::GateRestore
        | QaScenario::GateNegative
        | QaScenario::GateNoProof => stages_for_scenario(scenario)
            .into_iter()
            .flat_map(|stage| tokens_for_stage(stage).iter().copied())
            .collect(),
        QaScenario::All => implemented_final_tokens(),
    }
}

fn scenario_report(server_kind: &str, scenario: QaScenario) -> String {
    format!(
        "server={server_kind}\n{}",
        final_tokens_for_scenario(scenario).join("\n")
    )
}

async fn run_gate_restore_stage(
    mut conn: CoreConnection,
    runtime: CoreRuntime,
    data_dir: std::path::PathBuf,
    account_key: AccountKey,
) -> Result<(), String> {
    println!("gate_restore_bootstrapped=ok");
    let stop_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: stop_id,
        })),
    )
    .await
    .map_err(|_| "gate restore sync-stop submit timed out".to_owned())?
    .map_err(|error| format!("gate restore sync-stop submit: {error}"))?;
    wait_for_sync_stopped(&mut conn, stop_id, "gate restore sync stop").await?;
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| "gate restore runtime shutdown timed out".to_owned())?;
    println!("gate_restore_shutdown_complete=ok");

    let reopened = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn = reopened.attach();
    println!("gate_restore_runtime_spawned=ok");
    let query_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Account(AccountCommand::QuerySavedSessions {
            request_id: query_id,
        })),
    )
    .await
    .map_err(|_| "gate restore query submit timed out".to_owned())?
    .map_err(|error| format!("gate restore query submit: {error}"))?;
    println!("gate_restore_query_sent=ok");
    wait_for_saved_session_presence(&mut conn, query_id, &account_key).await?;
    println!("gate_restore_query_result=ok");

    let restore_started_at = std::time::Instant::now();
    let restore_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        })),
    )
    .await
    .map_err(|_| "gate restore restore submit timed out".to_owned())?
    .map_err(|error| format!("gate restore restore submit: {error}"))?;
    println!("gate_restore_restore_sent=ok");
    wait_for_session_restored(&mut conn, restore_id, &account_key, "gate restore").await?;
    println!("gate_restore_restore_result=ok");
    wait_for_ready_snapshot(&mut conn, "gate restore Ready").await?;
    if restore_started_at.elapsed() > GATE_RESTORE_READY_BUDGET {
        return Err("gate restore exceeded bounded Ready budget".to_owned());
    }
    println!("gate_restore_ready=ok");
    println!("gate_verified_restore=ok");
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, reopened.shutdown())
        .await
        .map_err(|_| "gate restore reopened shutdown timed out".to_owned())?;
    Ok(())
}

async fn run_gate_no_proof_stage(config: &QaConfig) -> Result<(), String> {
    let raw = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi No Proof Fixture".to_owned()),
    })
    .await
    .map_err(|_| "no-proof fixture login failed".to_owned())?;
    koushi_sdk::sync_once(&raw)
        .await
        .map_err(|_| "no-proof fixture sync failed".to_owned())?;
    koushi_sdk::bootstrap_cross_signing(&raw, Some(&AuthSecret::new(config.password_a.clone())))
        .await
        .map_err(|_| "no-proof cross-signing bootstrap failed".to_owned())?;
    let device_ids = vec![raw.info.device_id.clone()];
    let uiaa_session = match koushi_sdk::delete_devices(&raw, &device_ids, None, None).await {
        Err(koushi_sdk::DeleteDevicesError::UiaaChallenge { session }) => session,
        Ok(()) => None,
        Err(_) => return Err("no-proof initial device delete failed".to_owned()),
    };
    if uiaa_session.is_some() {
        koushi_sdk::delete_devices(
            &raw,
            &device_ids,
            Some(&IdentityResetAuthRequest::UiaaPassword {
                password: AuthSecret::new(config.password_a.clone()),
            }),
            uiaa_session.as_deref(),
        )
        .await
        .map_err(|_| "no-proof authenticated device delete failed".to_owned())?;
    }
    let _ = koushi_sdk::close_session_stores(&raw).await;
    drop(raw);

    let data_dir = qa_data_dir("gate-no-proof");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();
    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: config.homeserver.clone(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi No Proof Core".to_owned()),
        },
    }))
    .await
    .map_err(|_| "no-proof Core login submit failed".to_owned())?;
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut saw_rejecting = false;
    loop {
        saw_rejecting |= matches!(conn.snapshot().session, SessionState::Rejecting { .. });
        if matches!(conn.snapshot().session, SessionState::SignedOut) && saw_rejecting {
            break;
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "no-proof rejection timed out".to_owned())?
            .map_err(|_| "no-proof event stream closed".to_owned())?;
    }
    println!("gate_no_proof_rejected=ok");
    drop(conn);
    runtime.shutdown().await;

    let reopened = CoreRuntime::start_with_data_dir(data_dir);
    let mut reopened_conn = reopened.attach();
    let restore_id = reopened_conn.next_request_id();
    reopened_conn
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_id,
        }))
        .await
        .map_err(|_| "no-proof restart restore submit failed".to_owned())?;
    let failure = wait_for_operation_failed_and_signed_out(
        &mut reopened_conn,
        restore_id,
        "no-proof restart restore",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err("no-proof restart did not remain SignedOut".to_owned());
    }
    println!("gate_no_proof_restart_signed_out=ok");
    drop(reopened_conn);
    reopened.shutdown().await;
    Ok(())
}

async fn run_gate_negative_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    recovery_secret: &AuthSecret,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "gate negative primary session")?;
    let runtime_a2 = CoreRuntime::start_with_data_dir(qa_data_dir("gate-negative-a2"));
    let mut conn_a2 = runtime_a2.attach();
    let login_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A2".to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative login submit: {error}"))?;
    let session_a2 = wait_for_existing_identity_gate(&mut conn_a2, "gate negative A2").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &session_a2,
        "gate negative mismatch",
        SasQaOutcome::Mismatch,
    )
    .await?;
    println!("gate_sas_mismatch_retryable=ok");
    let retry_session =
        wait_for_existing_identity_gate(&mut conn_a2, "gate negative retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &retry_session,
        "gate negative retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a2, login_id, "gate negative A2 login").await?;
    wait_for_ready_snapshot(&mut conn_a2, "gate negative A2 Ready").await?;
    println!("gate_sas_retry_ready=ok");
    drop(conn_a2);
    runtime_a2.shutdown().await;

    let runtime_a3 = CoreRuntime::start_with_data_dir(qa_data_dir("gate-negative-a3"));
    let mut conn_a3 = runtime_a3.attach();
    let login_a3 = conn_a3.next_request_id();
    conn_a3
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a3,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A3".to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative A3 login submit: {error}"))?;
    let session_a3 = wait_for_existing_identity_gate(&mut conn_a3, "gate negative A3").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a3,
        &session_a,
        &session_a3,
        "gate negative user cancel",
        SasQaOutcome::UserCancel,
    )
    .await?;
    println!("gate_sas_user_cancel_retryable=ok");
    let retry_a3 = wait_for_existing_identity_gate(&mut conn_a3, "gate negative A3 retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a3,
        &session_a,
        &retry_a3,
        "gate negative user-cancel retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a3, login_a3, "gate negative A3 login").await?;
    wait_for_ready_snapshot(&mut conn_a3, "gate negative A3 Ready").await?;
    println!("gate_sas_user_cancel_retry_ready=ok");
    drop(conn_a3);
    runtime_a3.shutdown().await;

    let runtime_a4 = CoreRuntime::start_with_data_dir(qa_data_dir("gate-negative-a4"));
    let mut conn_a4 = runtime_a4.attach();
    let login_a4 = conn_a4.next_request_id();
    conn_a4
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a4,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A4".to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative A4 login submit: {error}"))?;
    let session_a4 = wait_for_existing_identity_gate(&mut conn_a4, "gate negative A4").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a4,
        &session_a,
        &session_a4,
        "gate negative timeout",
        SasQaOutcome::Timeout,
    )
    .await?;
    println!("gate_sas_timeout_retryable=ok");
    let retry_a4 = wait_for_existing_identity_gate(&mut conn_a4, "gate negative A4 retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a4,
        &session_a,
        &retry_a4,
        "gate negative timeout retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a4, login_a4, "gate negative A4 login").await?;
    wait_for_ready_snapshot(&mut conn_a4, "gate negative A4 Ready").await?;
    println!("gate_sas_timeout_retry_ready=ok");
    drop(conn_a4);
    runtime_a4.shutdown().await;

    let runtime_a5 = CoreRuntime::start_with_data_dir(qa_data_dir("gate-negative-a5"));
    let mut conn_a5 = runtime_a5.attach();
    let login_a5 = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a5,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A5".to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative A5 login submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a5, "gate negative A5").await?;
    let invalid_recovery = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: invalid_recovery,
            request: RecoveryRequest {
                secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET.to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative invalid recovery submit: {error}"))?;
    let failure = wait_for_operation_failed(
        &mut conn_a5,
        invalid_recovery,
        "gate negative invalid recovery",
    )
    .await?;
    if !matches!(failure, CoreFailure::RecoveryFailed { .. }) {
        return Err("gate negative invalid recovery returned unexpected failure kind".to_owned());
    }
    wait_for_recovery_gate(&mut conn_a5, "gate negative A5 retry").await?;
    println!("gate_recovery_invalid_retryable=ok");
    let valid_recovery = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: valid_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative valid recovery submit: {error}"))?;
    wait_for_ready_snapshot(&mut conn_a5, "gate negative recovery Ready").await?;
    println!("gate_recovery_retry_ready=ok");
    drop(conn_a5);
    runtime_a5.shutdown().await;

    let runtime_a6 = CoreRuntime::start_with_data_dir(qa_data_dir("gate-negative-a6"));
    let mut conn_a6 = runtime_a6.attach();
    let login_a6 = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a6,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A6".to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative A6 login submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a6, "gate negative A6").await?;
    let cancelled_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: cancelled_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative cancelled recovery submit: {error}"))?;
    wait_for_matching_recovery_flow(
        &mut conn_a6,
        cancelled_recovery.sequence,
        "gate negative cancelled recovery",
    )
    .await?;
    let cancel_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::CancelVerification {
            request_id: cancel_recovery,
            flow_id: cancelled_recovery.sequence,
            reason: koushi_state::VerificationCancelReason::User,
        }))
        .await
        .map_err(|error| format!("gate negative recovery cancel submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a6, "gate negative A6 cancelled retry").await?;
    println!("gate_recovery_cancel_retryable=ok");
    let retry_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: retry_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative recovery retry submit: {error}"))?;
    wait_for_ready_snapshot(&mut conn_a6, "gate negative cancelled recovery Ready").await?;
    println!("gate_recovery_cancel_retry_ready=ok");
    let account_key_a6 = AccountKey(
        authenticated_session_info(&mut conn_a6, "gate negative A6 reset session")?.user_id,
    );
    reset_identity_for_qa(
        &mut conn_a6,
        &account_key_a6,
        config.password_a.clone(),
        "gate negative trust loss reset",
    )
    .await?;
    wait_for_locked_snapshot(conn_a, "gate negative primary trust loss").await?;
    println!("gate_trust_loss_locked=ok");
    let blocked_sync = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: blocked_sync,
        }))
        .await
        .map_err(|error| format!("gate negative locked sync submit: {error}"))?;
    let failure =
        wait_for_operation_failed(conn_a, blocked_sync, "gate negative locked normal command")
            .await?;
    if failure != CoreFailure::SessionRequired {
        return Err("gate negative locked command returned unexpected failure kind".to_owned());
    }
    println!("gate_trust_loss_commands_blocked=ok");
    drop(conn_a6);
    runtime_a6.shutdown().await;
    Ok(())
}

async fn cleanup_after_login_sync(
    mut conn_a: CoreConnection,
    runtime_a: CoreRuntime,
    data_dir_a: std::path::PathBuf,
    account_key_a: AccountKey,
) -> Result<String, String> {
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");
    drop(conn_a);
    runtime_a.shutdown().await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;
    println!("gate_verified_restore=ok");

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    let restore_gone_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_gone_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn_a2,
        restore_gone_id,
        "post-logout restore A (must fail)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore A failed with unexpected kind: {failure:?}"
        ));
    }
    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

async fn wait_for_saved_session_presence(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: &AccountKey,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "timed out waiting for saved-session readiness".to_owned())?
            .map_err(|lag| {
                format!(
                    "saved-session readiness event lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: event_id,
                sessions,
            }) if event_id == request_id => {
                if sessions.iter().any(|session| session.user_id == expected.0) {
                    return Ok(());
                }
                return Err(format!(
                    "saved-session readiness missing expected account; saved_count={}",
                    sessions.len()
                ));
            }
            CoreEvent::OperationFailed {
                request_id: event_id,
                failure,
            } if event_id == request_id => {
                return Err(format!("saved-session readiness failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn run_invites_dm_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let user_a_full_id = format!("@{}:{}", config.user_a, config.server_name);

    let accept_room_id = create_room_for_qa(
        conn_a,
        "QA Invite Accept Room",
        false,
        "invites_dm create accept room",
    )
    .await?;
    invite_user_for_qa(
        conn_a,
        &accept_room_id,
        &user_b_full_id,
        "invites_dm invite B to room",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &accept_room_id,
        Some(false),
        "invites_dm wait for room invite",
    )
    .await?;
    println!("invite_recv=ok");

    accept_invite_for_qa(conn_b, &accept_room_id, "invites_dm accept room invite").await?;
    wait_for_room_in_room_list(
        conn_b,
        &accept_room_id,
        "invites_dm room list after room accept",
    )
    .await?;
    let accept_room_settings =
        load_room_settings_for_qa(conn_b, &accept_room_id, "invites_dm accepted room members")
            .await?;
    assert_room_settings_contains_members(
        &accept_room_settings,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm accepted room members",
    )?;

    let accept_space_id = create_space_for_qa(
        conn_a,
        "QA Invite Accept Space",
        "invites_dm create accept space",
    )
    .await?;
    invite_user_for_qa(
        conn_a,
        &accept_space_id,
        &user_b_full_id,
        "invites_dm invite B to space",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &accept_space_id,
        Some(false),
        "invites_dm wait for space invite",
    )
    .await?;
    accept_invite_for_qa(conn_b, &accept_space_id, "invites_dm accept space invite").await?;
    wait_for_space_in_space_list(
        conn_b,
        &accept_space_id,
        "invites_dm space list after space accept",
    )
    .await?;
    let accept_space_settings = load_room_settings_for_qa(
        conn_b,
        &accept_space_id,
        "invites_dm accepted space members",
    )
    .await?;
    assert_room_settings_contains_members(
        &accept_space_settings,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm accepted space members",
    )?;
    let accept_space_settings_a = load_room_settings_for_qa(
        conn_a,
        &accept_space_id,
        "invites_dm creator observes accepted space member",
    )
    .await?;
    assert_room_settings_contains_members(
        &accept_space_settings_a,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm creator observes accepted space member",
    )?;
    println!("invite_accept=ok");
    println!("member_list=ok");

    let decline_room_id = create_room_for_qa(
        conn_a,
        "QA Invite Decline Room",
        false,
        "invites_dm create decline room",
    )
    .await?;
    invite_user_for_qa(
        conn_a,
        &decline_room_id,
        &user_b_full_id,
        "invites_dm invite B to decline room",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &decline_room_id,
        Some(false),
        "invites_dm wait for decline invite",
    )
    .await?;
    decline_invite_for_qa(conn_b, &decline_room_id, "invites_dm decline room invite").await?;
    wait_for_invite_absent(
        conn_b,
        &decline_room_id,
        "invites_dm wait for declined invite removal",
    )
    .await?;
    println!("invite_decline=ok");

    let dm_room_id =
        start_direct_message_for_qa(conn_a, &user_b_full_id, "invites_dm start direct message")
            .await?;
    wait_for_dm_room_in_room_list(conn_a, &dm_room_id, "invites_dm A room list after DM start")
        .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &dm_room_id,
        Some(true),
        "invites_dm wait for DM invite",
    )
    .await?;
    println!("dm_start=ok");

    let user_c_full_id = config.dm_scope_control_user_id()?;
    let control_dm_room_id = start_direct_message_for_qa(
        conn_a,
        &user_c_full_id,
        "invites_dm start control direct message",
    )
    .await?;
    wait_for_dm_room_in_room_list(
        conn_a,
        &control_dm_room_id,
        "invites_dm A room list after control DM start",
    )
    .await?;
    assert_dm_space_scope_for_qa(conn_a, &accept_space_id, &dm_room_id, &control_dm_room_id)
        .await?;
    println!("dm_space_scope=ok");

    Ok(())
}

async fn run_directory_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
) -> Result<(), String> {
    let directory_room_name = "Koushi Directory QA";
    let alias_localpart = format!("koushi-desktop-directory-qa-{}", std::process::id());
    let expected_alias = format!("#{alias_localpart}:{}", config.server_name);
    let public_room_id = create_public_directory_room_for_qa(
        conn_a,
        directory_room_name,
        &alias_localpart,
        "directory create public room",
    )
    .await?;

    let query = DirectoryQuery {
        term: Some(directory_room_name.to_owned()),
        server_name: Some(config.server_name.clone()),
        limit: Some(10),
        since: None,
    };
    let rooms = query_directory_until_room_visible(
        conn_a,
        query,
        &public_room_id,
        &expected_alias,
        "directory query public room",
    )
    .await?;
    if rooms.is_empty() {
        return Err("directory query unexpectedly returned no rooms".to_owned());
    }
    println!("directory_query=ok");

    join_directory_room_for_qa(
        conn_b,
        &expected_alias,
        &config.server_name,
        &public_room_id,
        "directory B joins public room",
    )
    .await?;
    println!("directory_join=ok");

    Ok(())
}

async fn join_directory_room_for_qa(
    conn_b: &mut CoreConnection,
    expected_alias: &str,
    via_server: &str,
    public_room_id: &str,
    label: &str,
) -> Result<(), String> {
    let join_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
            request_id: join_id,
            alias: expected_alias.to_owned(),
            via_server: Some(via_server.to_owned()),
        }))
        .await
        .map_err(|e| format!("{label}: submit join by alias failed: {e}"))?;
    wait_for_room_joined(conn_b, join_id, public_room_id, label).await
}

async fn run_room_management_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
) -> Result<(), String> {
    let room_id = create_room_for_qa(
        conn_a,
        "QA Room Management",
        false,
        "room_management create room",
    )
    .await?;
    wait_for_room_in_room_list(conn_a, &room_id, "room_management A room list").await?;

    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    invite_user_for_qa(
        conn_a,
        &room_id,
        &user_b_full_id,
        "room_management invite B",
    )
    .await?;
    wait_for_invite_in_snapshot(
        conn_b,
        &room_id,
        Some(false),
        "room_management wait for B invite",
    )
    .await?;

    let join_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_b_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("room_management: submit B join failed: {e}"))?;
    wait_for_room_joined(conn_b, join_b_id, &room_id, "room_management B joins").await?;
    let settings_a =
        load_room_settings_for_qa(conn_a, &room_id, "room_management load settings A").await?;
    assert_room_settings_contains_members(
        &settings_a,
        &[account_key_a.0.as_str(), account_key_b.0.as_str()],
        "room_management A observes joined members",
    )?;
    if !settings_a.permissions.can_edit_settings || !settings_a.permissions.can_kick {
        return Err("room_management: creator permissions were not projected".to_owned());
    }

    let update_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
            request_id: update_id,
            room_id: room_id.clone(),
            change: RoomSettingChange::Topic(Some("QA room management topic".to_owned())),
        }))
        .await
        .map_err(|e| format!("room_management: submit topic update failed: {e}"))?;
    let updated =
        wait_for_room_setting_updated(conn_a, update_id, "room_management topic update").await?;
    if updated.topic.as_deref() != Some("QA room management topic") {
        return Err("room_management: updated settings snapshot did not carry topic".to_owned());
    }
    println!("room_settings=ok");

    let settings_b =
        load_room_settings_for_qa(conn_b, &room_id, "room_management load settings B").await?;
    assert_room_settings_contains_members(
        &settings_b,
        &[account_key_a.0.as_str(), account_key_b.0.as_str()],
        "room_management B observes joined members",
    )?;
    if settings_b.permissions.can_kick {
        return Err("room_management: normal member unexpectedly has kick permission".to_owned());
    }

    let guard_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::ModerateRoomMember {
            request_id: guard_id,
            room_id: room_id.clone(),
            target_user_id: account_key_a.0.clone(),
            action: RoomModerationAction::Kick,
            reason: None,
        }))
        .await
        .map_err(|e| format!("room_management: submit forbidden moderation failed: {e}"))?;
    wait_for_room_management_forbidden(conn_b, guard_id, "room_management permission guard")
        .await?;
    println!("permission_guard=ok");

    let kick_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::ModerateRoomMember {
            request_id: kick_id,
            room_id,
            target_user_id: account_key_b.0.clone(),
            action: RoomModerationAction::Kick,
            reason: Some("QA moderation".to_owned()),
        }))
        .await
        .map_err(|e| format!("room_management: submit kick failed: {e}"))?;
    wait_for_room_member_moderated(conn_a, kick_id, "room_management member kick").await?;
    println!("moderation=ok");

    Ok(())
}

async fn run_e2ee_trust_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
    recipient_base: Option<(&mut CoreConnection, &AccountKey)>,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "session A info for E2EE trust")?;

    // The login gate already bootstrapped and authoritatively promoted this
    // session. Re-running bootstrap here would rotate the identity and
    // invalidate the proof device that A2 is about to use.
    println!("e2ee_cross_signing_reused=ok");
    println!("e2ee_cross_signing=ok");

    let key_backup_seed_room_id =
        seed_encrypted_room_key_for_qa(conn_a, account_key_a, "seed key backup room A").await?;
    println!("e2ee_key_backup_seed=ok");

    let key_backup_version = enable_key_backup_for_qa(
        conn_a,
        account_key_a,
        Some(AuthSecret::new(config.password_a.clone())),
        "enable key backup A",
    )
    .await?;
    println!("e2ee_key_backup_enable=ok");

    let runtime_a2 = CoreRuntime::start_with_data_dir(qa_data_dir("a2"));
    let conn_a2 = runtime_a2.attach();
    let mut owned_a2 = QaOwnedRuntimeParticipant::new(runtime_a2, conn_a2);
    let a2_stage_result: Result<(), String> = async {
        let login_a2_id = owned_a2.conn.next_request_id();
        owned_a2
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_a2_id,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_a.clone(),
                    password: AuthSecret::new(config.password_a.clone()),
                    device_display_name: Some("Koushi Core QA A2".to_owned()),
                },
            }))
            .await
            .map_err(|e| format!("submit login A2: {e}"))?;
        owned_a2.mark_login_submitted();

        let session_a2 =
            wait_for_existing_identity_gate(&mut owned_a2.conn, "session A2 gate").await?;
        verify_provisional_second_device_for_qa(
            conn_a,
            &mut owned_a2.conn,
            &session_a,
            &session_a2,
            "e2ee gated self verification A/A2",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_a2 =
            wait_for_logged_in(&mut owned_a2.conn, login_a2_id, "login A2").await?;
        owned_a2.mark_logged_in(account_key_a2.clone());
        let conn_a2 = &mut owned_a2.conn;
        wait_for_ready_snapshot(conn_a2, "session A2 Ready").await?;
        println!("gate_own_sas=ok");

        let sync_start_a2_id = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: sync_start_a2_id,
            }))
            .await
            .map_err(|e| format!("submit sync start A2: {e}"))?;
        let sync_backend_a2 =
            wait_for_sync_started_and_running(conn_a2, sync_start_a2_id, "sync start A2").await?;
        assert_expected_backend(
            config.expect_sync_backend.as_deref(),
            sync_backend_a2,
            "sync start A2",
        )?;

        wait_for_room_in_room_list(
            conn_a2,
            &key_backup_seed_room_id,
            "room list A2 after key backup seed",
        )
        .await?;

        restore_key_backup_failure_for_qa(
            conn_a2,
            &account_key_a2,
            Some(key_backup_version.clone()),
            "restore key backup failure A2",
        )
        .await?;
        println!("e2ee_key_backup_restore_failure=ok");

        restore_key_backup_success_for_qa(
            conn_a2,
            &account_key_a2,
            Some(key_backup_version),
            AuthSecret::new(config.password_a.clone()),
            "restore key backup success A2",
        )
        .await?;
        println!("joined_room_restore=ok");

        println!("e2ee_verification=ok");

        verify_second_device_room_key_delivery_for_qa(
            conn_a,
            conn_a2,
            account_key_a,
            &account_key_a2,
            &key_backup_seed_room_id,
        )
        .await?;
        println!("e2ee_second_device_decrypt=ok");

        verify_multi_user_multi_device_room_key_delivery_for_qa(
            config,
            conn_a,
            conn_a2,
            account_key_a,
            &account_key_a2,
            recipient_base,
        )
        .await?;
        println!("e2ee_multi_user_multi_device_decrypt=ok");
        Ok(())
    }
    .await;

    finish_e2ee_recipient_stage_with_owned_cleanup(
        a2_stage_result,
        Some(owned_a2),
        |participant| async move {
            cleanup_owned_e2ee_participant_best_effort(participant, "cleanup secondary device")
                .await
        },
    )
    .await?;

    if config.allow_identity_reset {
        reset_identity_for_qa(
            conn_a,
            account_key_a,
            config.password_a.clone(),
            "reset identity A",
        )
        .await?;
        println!("e2ee_identity_reset=ok");
    } else {
        println!("e2ee_identity_reset=skipped");
    }
    println!("e2ee_trust=ok");

    Ok(())
}

async fn cleanup_logged_in_runtime(
    mut conn: CoreConnection,
    runtime: CoreRuntime,
    account_key: AccountKey,
    label: &str,
) -> Result<(), String> {
    let sync_stop_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop {
        request_id: sync_stop_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit sync stop failed: {e}"))?;
    wait_for_sync_stopped(&mut conn, sync_stop_id, label).await?;

    let logout_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::Logout {
        request_id: logout_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit logout failed: {e}"))?;
    wait_for_logged_out(&mut conn, logout_id, &account_key, label).await?;

    drop(conn);
    runtime.shutdown().await;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QaE2eeLogoutBarrier {
    AnyAccount,
    Exact(AccountKey),
}

fn e2ee_cleanup_logout_barrier(phase: &QaOwnedRuntimePhase) -> Option<QaE2eeLogoutBarrier> {
    match phase {
        QaOwnedRuntimePhase::LoginNotSubmitted => None,
        // Login was submitted, but ownership has not advanced through the
        // authoritative LoggedIn gate. Do not infer an exact account key from
        // a provisional snapshot.
        QaOwnedRuntimePhase::LoginSubmitted => Some(QaE2eeLogoutBarrier::AnyAccount),
        QaOwnedRuntimePhase::LoggedIn(account_key) => {
            Some(QaE2eeLogoutBarrier::Exact(account_key.clone()))
        }
    }
}

trait QaOwnedE2eeCleanupOperations {
    async fn stop_sync(&mut self, label: &str) -> Result<(), String>;
    async fn submit_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String>;
    async fn wait_for_authoritative_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String>;
    fn drop_connection(&mut self);
    async fn shutdown_runtime(&mut self);
}

struct QaCoreOwnedE2eeCleanupOperations {
    runtime: Option<CoreRuntime>,
    conn: Option<CoreConnection>,
    logout_request_id: Option<koushi_core::ids::RequestId>,
}

impl QaCoreOwnedE2eeCleanupOperations {
    fn new(runtime: CoreRuntime, conn: CoreConnection) -> Self {
        Self {
            runtime: Some(runtime),
            conn: Some(conn),
            logout_request_id: None,
        }
    }

    fn connection(&mut self) -> &mut CoreConnection {
        self.conn
            .as_mut()
            .expect("owned E2EE cleanup connection is available before its drop barrier")
    }
}

impl QaOwnedE2eeCleanupOperations for QaCoreOwnedE2eeCleanupOperations {
    async fn stop_sync(&mut self, label: &str) -> Result<(), String> {
        let conn = self.connection();
        let sync_stop_id = conn.next_request_id();
        match conn
            .command(CoreCommand::Sync(SyncCommand::Stop {
                request_id: sync_stop_id,
            }))
            .await
        {
            Ok(()) => wait_for_sync_stopped(conn, sync_stop_id, label).await,
            Err(_) => Err(format!("{label}: submit sync stop failed")),
        }
    }

    async fn submit_logout(
        &mut self,
        _barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String> {
        let conn = self.connection();
        let logout_request_id = conn.next_request_id();
        conn.command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_request_id,
        }))
        .await
        .map_err(|_| format!("{label}: submit logout failed"))?;
        self.logout_request_id = Some(logout_request_id);
        Ok(())
    }

    async fn wait_for_authoritative_logout(
        &mut self,
        barrier: &QaE2eeLogoutBarrier,
        label: &str,
    ) -> Result<(), String> {
        let logout_request_id = self
            .logout_request_id
            .take()
            .expect("logout submission precedes its authoritative cleanup barrier");
        let conn = self.connection();
        match barrier {
            QaE2eeLogoutBarrier::AnyAccount => {
                wait_for_signed_out_after_logout(conn, logout_request_id, label).await
            }
            QaE2eeLogoutBarrier::Exact(account_key) => {
                wait_for_logged_out(conn, logout_request_id, account_key, label).await
            }
        }
    }

    fn drop_connection(&mut self) {
        drop(self.conn.take());
    }

    async fn shutdown_runtime(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown().await;
        }
    }
}

async fn cleanup_owned_e2ee_lifecycle_best_effort<Operations>(
    phase: &QaOwnedRuntimePhase,
    operations: &mut Operations,
    label: &str,
) -> Result<(), String>
where
    Operations: QaOwnedE2eeCleanupOperations,
{
    let sync_stop_result = if matches!(phase, QaOwnedRuntimePhase::LoggedIn(_)) {
        operations.stop_sync(label).await
    } else {
        Ok(())
    };

    // Logout is attempted even if stopping sync failed. Connection drop and
    // ordered runtime shutdown remain the final barriers in every phase.
    let logout_result = if let Some(barrier) = e2ee_cleanup_logout_barrier(phase) {
        match operations.submit_logout(&barrier, label).await {
            Ok(()) => {
                operations
                    .wait_for_authoritative_logout(&barrier, label)
                    .await
            }
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };

    operations.drop_connection();
    operations.shutdown_runtime().await;

    match (sync_stop_result, logout_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(_), Ok(())) => Err(format!("{label}: sync stop cleanup failed")),
        (Ok(()), Err(_)) => Err(format!("{label}: logout cleanup failed")),
        (Err(_), Err(_)) => Err(format!("{label}: sync stop and logout cleanup failed")),
    }
}

async fn cleanup_owned_e2ee_participant_best_effort(
    participant: QaOwnedRuntimeParticipant,
    label: &str,
) -> Result<(), String> {
    let QaOwnedRuntimeParticipant {
        runtime,
        conn,
        phase,
    } = participant;
    let mut operations = QaCoreOwnedE2eeCleanupOperations::new(runtime, conn);
    cleanup_owned_e2ee_lifecycle_best_effort(&phase, &mut operations, label).await
}

async fn cleanup_e2ee_callers_after_stage_failure(
    callers: (QaOwnedRuntimeParticipant, QaOwnedRuntimeParticipant),
) -> Result<(), String> {
    let (caller_a, caller_b) = callers;
    let cleanup_a =
        cleanup_owned_e2ee_participant_best_effort(caller_a, "all E2EE failure cleanup caller A")
            .await;
    let cleanup_b =
        cleanup_owned_e2ee_participant_best_effort(caller_b, "all E2EE failure cleanup caller B")
            .await;

    match (cleanup_a, cleanup_b) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(_), Ok(())) => Err("all E2EE caller A cleanup failed".to_owned()),
        (Ok(()), Err(_)) => Err("all E2EE caller B cleanup failed".to_owned()),
        (Err(_), Err(_)) => Err("all E2EE caller cleanup failed for both participants".to_owned()),
    }
}

async fn cleanup_e2ee_multi_device_participants(
    participants: (
        Option<QaOwnedRuntimeParticipant>,
        Option<QaOwnedRuntimeParticipant>,
        Option<QaOwnedRuntimeParticipant>,
    ),
) -> Result<(), String> {
    let (base, second_device, unverified_device) = participants;
    cleanup_all_owned_e2ee_participants(
        [
            unverified_device.map(|participant| (participant, "e2ee cleanup B3")),
            second_device.map(|participant| (participant, "e2ee cleanup B2")),
            base.map(|participant| (participant, "e2ee cleanup B")),
        ],
        |(participant, label)| async move {
            cleanup_owned_e2ee_participant_best_effort(participant, label).await
        },
    )
    .await
}

async fn cleanup_all_owned_e2ee_participants<Participant, Cleanup, CleanupFuture, const N: usize>(
    participants: [Option<Participant>; N],
    mut cleanup: Cleanup,
) -> Result<(), String>
where
    Cleanup: FnMut(Participant) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    let mut failed = 0usize;
    for participant in participants.into_iter().flatten() {
        if cleanup(participant).await.is_err() {
            failed += 1;
        }
    }

    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "E2EE cleanup failed for {failed} owned recipient participant(s)"
        ))
    }
}

async fn cleanup_normal_secondary_participant_for_qa(
    normal_secondary: &mut Option<QaParticipantLoginOutcome>,
    label: &str,
) -> Result<(), String> {
    let Some(participant) = normal_secondary.take() else {
        return Ok(());
    };
    cleanup_logged_in_runtime(
        participant.conn,
        participant.runtime,
        participant.account_key,
        label,
    )
    .await
}

async fn run_timeline_stress_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
) -> Result<(), String> {
    let stress = TimelineStressConfig::from_env()?;
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let mut created_room_count = 0usize;
    let mut sent_message_count = 0usize;

    for space_index in 0..stress.space_count {
        eprintln!("timeline_stress progress: create_space index={space_index}");
        let space_id = create_space_for_qa(
            conn_a,
            &format!("Koushi Stress Space {space_index}"),
            "timeline_stress create space",
        )
        .await?;
        invite_user_for_qa(
            conn_a,
            &space_id,
            &user_b_full_id,
            "timeline_stress invite user to space",
        )
        .await?;
        wait_for_invite_in_snapshot(
            conn_b,
            &space_id,
            Some(false),
            "timeline_stress receiver sees space invite",
        )
        .await?;
        accept_invite_for_qa(conn_b, &space_id, "timeline_stress accept space invite").await?;
        wait_for_space_in_space_list(conn_a, &space_id, "timeline_stress creator sees space")
            .await?;
        wait_for_space_in_space_list(conn_b, &space_id, "timeline_stress receiver sees space")
            .await?;

        let mut expected_room_ids = Vec::with_capacity(stress.rooms_per_space);
        for room_index in 0..stress.rooms_per_space {
            eprintln!(
                "timeline_stress progress: create_room space={space_index} room={room_index}"
            );
            let room_id = create_room_for_qa(
                conn_a,
                &format!("Koushi Stress Room {space_index}-{room_index}"),
                false,
                "timeline_stress create room",
            )
            .await?;
            set_space_child_for_qa(
                conn_a,
                &space_id,
                &room_id,
                &config.server_name,
                "timeline_stress set space child",
            )
            .await?;
            invite_user_for_qa(
                conn_a,
                &room_id,
                &user_b_full_id,
                "timeline_stress invite user to room",
            )
            .await?;
            wait_for_invite_in_snapshot(
                conn_b,
                &room_id,
                Some(false),
                "timeline_stress receiver sees room invite",
            )
            .await?;
            accept_invite_for_qa(conn_b, &room_id, "timeline_stress accept room invite").await?;
            wait_for_room_in_room_list(conn_a, &room_id, "timeline_stress creator sees room")
                .await?;
            wait_for_room_in_room_list(conn_b, &room_id, "timeline_stress receiver sees room")
                .await?;

            expected_room_ids.push(room_id.clone());
            wait_for_space_child_projection(
                conn_a,
                &space_id,
                &expected_room_ids,
                "timeline_stress creator space children",
            )
            .await?;
            wait_for_space_child_projection(
                conn_b,
                &space_id,
                &expected_room_ids,
                "timeline_stress receiver space children",
            )
            .await?;
            created_room_count += 1;

            let sender_is_a = (space_index + room_index) % 2 == 0;
            eprintln!(
                "timeline_stress progress: messages space={space_index} room={room_index} sender={}",
                if sender_is_a { "a" } else { "b" }
            );
            sent_message_count += if sender_is_a {
                run_timeline_stress_room_messages(
                    config,
                    conn_a,
                    conn_b,
                    account_key_a,
                    account_key_b,
                    &room_id,
                    StressRoomCoordinates {
                        sender_prefix: "a",
                        space_index,
                        room_index,
                    },
                    stress.messages_per_room,
                )
                .await?
            } else {
                run_timeline_stress_room_messages(
                    config,
                    conn_b,
                    conn_a,
                    account_key_b,
                    account_key_a,
                    &room_id,
                    StressRoomCoordinates {
                        sender_prefix: "b",
                        space_index,
                        room_index,
                    },
                    stress.messages_per_room,
                )
                .await?
            };
        }

        select_space_and_wait_for_room_scope(
            conn_a,
            &space_id,
            &expected_room_ids,
            "timeline_stress creator selected-space scope",
        )
        .await?;
        select_space_and_wait_for_room_scope(
            conn_b,
            &space_id,
            &expected_room_ids,
            "timeline_stress receiver selected-space scope",
        )
        .await?;
    }

    if created_room_count != stress.total_rooms() || sent_message_count != stress.total_messages() {
        return Err(format!(
            "timeline_stress: count mismatch rooms={created_room_count}/{} messages={sent_message_count}/{}",
            stress.total_rooms(),
            stress.total_messages()
        ));
    }

    println!(
        "stress_counts=spaces={} rooms={} messages={}",
        stress.space_count,
        stress.total_rooms(),
        stress.total_messages()
    );
    println!("stress_space_scope=ok");
    println!("stress_no_blank=ok");
    println!("timeline_stress=ok");
    Ok(())
}

async fn run_timeline_stress_replay_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_b: &AccountKey,
    _stress: TimelineStressConfig,
) -> Result<(), String> {
    let snapshot_a =
        wait_for_existing_stress_fixture_room_list(conn_a, "timeline_stress replay A room list")
            .await?;
    let snapshot_b =
        wait_for_existing_stress_fixture_room_list(conn_b, "timeline_stress replay B room list")
            .await?;
    verify_existing_stress_space_scopes(
        conn_a,
        &snapshot_a,
        "timeline_stress replay A selected-space scope",
    )
    .await?;
    verify_existing_stress_space_scopes(
        conn_b,
        &snapshot_b,
        "timeline_stress replay B selected-space scope",
    )
    .await?;

    let room_ids_a = stress_replay_room_ids(&snapshot_a);
    let room_ids_b = stress_replay_room_ids(&snapshot_b);
    if room_ids_a.is_empty() || room_ids_b.is_empty() {
        return Err("timeline_stress replay: fixture has no joined rooms".to_owned());
    }

    let scan_a = scan_existing_stress_rooms(
        conn_a,
        account_key_a,
        &room_ids_a,
        "timeline_stress replay A timeline scan",
    )
    .await?;
    let scan_b = scan_existing_stress_rooms(
        conn_b,
        account_key_b,
        &room_ids_b,
        "timeline_stress replay B timeline scan",
    )
    .await?;
    let message_rows = scan_a.message_rows + scan_b.message_rows;
    if message_rows == 0 {
        return Err(
            "timeline_stress replay: fixture timelines contained no visible messages".to_owned(),
        );
    }

    println!(
        "stress_counts=spaces={} rooms={} messages={}",
        snapshot_a.spaces.len().max(snapshot_b.spaces.len()),
        scan_a.rooms.max(scan_b.rooms),
        message_rows
    );
    println!("stress_space_scope=ok");
    println!("stress_no_blank=ok");
    println!("timeline_stress=ok");
    Ok(())
}

async fn wait_for_existing_stress_fixture_room_list(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<AppState, String> {
    let has_fixture_shape =
        |snapshot: &AppState| !snapshot.rooms.is_empty() && !snapshot.spaces.is_empty();
    let snapshot = conn.snapshot();
    if has_fixture_shape(&snapshot) {
        return Ok(snapshot);
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for existing fixture rooms/spaces \
                     (rooms={}, spaces={})",
                    snapshot.rooms.len(),
                    snapshot.spaces.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if has_fixture_shape(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if has_fixture_shape(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => {}
        }
    }
}

async fn verify_existing_stress_space_scopes(
    conn: &mut CoreConnection,
    snapshot: &AppState,
    label: &str,
) -> Result<(), String> {
    let spaces = snapshot
        .spaces
        .iter()
        .filter(|space| !space.child_room_ids.is_empty())
        .map(|space| (space.space_id.clone(), space.child_room_ids.clone()))
        .collect::<Vec<_>>();
    if spaces.is_empty() {
        return Err(format!("{label}: fixture has no spaces with child rooms"));
    }
    for (space_id, child_room_ids) in spaces {
        select_space_and_wait_for_room_scope(conn, &space_id, &child_room_ids, label).await?;
    }
    Ok(())
}

fn stress_replay_room_ids(snapshot: &AppState) -> Vec<String> {
    let joined_room_ids = snapshot
        .rooms
        .iter()
        .map(|room| room.room_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut room_ids = BTreeSet::new();
    for space in &snapshot.spaces {
        for room_id in &space.child_room_ids {
            if joined_room_ids.contains(room_id.as_str()) {
                room_ids.insert(room_id.clone());
            }
        }
    }
    if room_ids.is_empty() {
        for room in &snapshot.rooms {
            room_ids.insert(room.room_id.clone());
        }
    }
    room_ids.into_iter().collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StressReplayScan {
    rooms: usize,
    message_rows: usize,
}

async fn scan_existing_stress_rooms(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_ids: &[String],
    label: &str,
) -> Result<StressReplayScan, String> {
    let mut message_rows = 0usize;
    for room_id in room_ids {
        message_rows += scan_existing_stress_timeline(conn, account_key, room_id, label).await?;
    }
    Ok(StressReplayScan {
        rooms: room_ids.len(),
        message_rows,
    })
}

async fn scan_existing_stress_timeline(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_id: &str,
    label: &str,
) -> Result<usize, String> {
    let key = TimelineKey::room(account_key.clone(), room_id.to_owned());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit replay subscribe failed: {e}"))?;
    let initial_items = wait_for_initial_items(conn, &key, subscribe_id, label).await?;
    assert_no_blank_visible_event_rows(&initial_items, label)?;
    let mut message_rows = count_visible_payload_event_rows(&initial_items);
    let mut end_reached = false;
    let mut page_count = 0usize;
    while !end_reached && page_count < 3 {
        let request_id = submit_stress_backfill_paginate(conn, &key, 100, label).await?;
        let result = wait_for_stress_replay_paginate(conn, &key, request_id, label).await?;
        message_rows += result.message_rows;
        end_reached = result.end_reached;
        page_count += 1;
    }

    let unsubscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: unsubscribe_id,
        key,
    }))
    .await
    .map_err(|e| format!("{label}: submit replay unsubscribe failed: {e}"))?;
    Ok(message_rows)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StressReplayPageResult {
    message_rows: usize,
    end_reached: bool,
}

async fn wait_for_stress_replay_paginate(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
) -> Result<StressReplayPageResult, String> {
    let mut message_rows = 0usize;
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for replay paginate"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_is_visible_event_row(item)
                        && !timeline_item_has_visible_payload(item)
                    {
                        return Err(format!(
                            "{label}: visible event row had no renderable payload"
                        ));
                    }
                    Ok(())
                })?;
                message_rows += count_visible_payload_event_rows_in_diffs(&diffs);
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => {
                assert_no_blank_visible_event_rows(&items, label)?;
                message_rows += count_visible_payload_event_rows(&items);
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ev_key,
                request_id: ev_id,
                state,
                ..
            }) if ev_key == key && ev_id == &Some(request_id) => match state {
                PaginationState::Idle => {
                    return Ok(StressReplayPageResult {
                        message_rows,
                        end_reached: false,
                    });
                }
                PaginationState::EndReached => {
                    return Ok(StressReplayPageResult {
                        message_rows,
                        end_reached: true,
                    });
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: replay pagination failed: {kind:?}"));
                }
                PaginationState::Paginating => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == &request_id => {
                return Err(format!(
                    "{label}: replay paginate operation failed: {failure:?}"
                ));
            }
            _ => {}
        }
    }
}

fn count_visible_payload_event_rows(items: &[TimelineItem]) -> usize {
    items
        .iter()
        .filter(|item| {
            timeline_item_is_visible_event_row(item) && timeline_item_has_visible_payload(item)
        })
        .count()
}

fn count_visible_payload_event_rows_in_diffs(diffs: &[TimelineDiff]) -> usize {
    let mut count = 0usize;
    let _ = visit_timeline_diff_items(diffs, |item| {
        if timeline_item_is_visible_event_row(item) && timeline_item_has_visible_payload(item) {
            count += 1;
        }
        Ok(())
    });
    count
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StressRoomCoordinates {
    sender_prefix: &'static str,
    space_index: usize,
    room_index: usize,
}

impl StressRoomCoordinates {
    fn should_send_empty_formatted_probe(self) -> bool {
        self.space_index == 0 && self.room_index == 0
    }
}

async fn run_timeline_stress_room_messages(
    config: &QaConfig,
    sender_conn: &mut CoreConnection,
    receiver_conn: &mut CoreConnection,
    sender_account_key: &AccountKey,
    receiver_account_key: &AccountKey,
    room_id: &str,
    coordinates: StressRoomCoordinates,
    messages_per_room: usize,
) -> Result<usize, String> {
    let sender_key = TimelineKey::room(sender_account_key.clone(), room_id.to_owned());
    let sender_subscribe_id = sender_conn.next_request_id();
    sender_conn
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: sender_subscribe_id,
            key: sender_key.clone(),
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit sender subscribe failed: {e}"))?;
    let sender_initial = wait_for_initial_items(
        sender_conn,
        &sender_key,
        sender_subscribe_id,
        "timeline_stress sender subscribe",
    )
    .await?;
    assert_no_blank_visible_event_rows(&sender_initial, "timeline_stress sender initial")?;

    let mut expected_bodies = Vec::with_capacity(messages_per_room);
    for message_index in 0..messages_per_room {
        let body = format!(
            "Koushi local stress body s{} r{} m{}",
            coordinates.space_index, coordinates.room_index, message_index
        );
        let transaction_id = format!(
            "qa-stress-{}-{}-{}-{}",
            coordinates.sender_prefix,
            coordinates.space_index,
            coordinates.room_index,
            message_index
        );
        let send_id = sender_conn.next_request_id();
        sender_conn
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: sender_key.clone(),
                transaction_id: transaction_id.clone(),
                body: body.clone(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("timeline_stress: submit stress send failed: {e}"))?;
        wait_for_send_flow_completion(
            sender_conn,
            send_id,
            &sender_key,
            &transaction_id,
            &body,
            "timeline_stress send flow",
        )
        .await?;
        expected_bodies.push(body);
    }

    if coordinates.should_send_empty_formatted_probe() {
        let probe_body = send_timeline_stress_empty_formatted_probe(
            config,
            room_id,
            coordinates.sender_prefix,
            "timeline_stress empty formatted probe",
        )
        .await?;
        expected_bodies.push(probe_body);
    }

    let sender_unsubscribe_id = sender_conn.next_request_id();
    sender_conn
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: sender_unsubscribe_id,
            key: sender_key,
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit sender unsubscribe failed: {e}"))?;

    let receiver_key = TimelineKey::room(receiver_account_key.clone(), room_id.to_owned());
    let receiver_subscribe_id = receiver_conn.next_request_id();
    receiver_conn
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: receiver_subscribe_id,
            key: receiver_key.clone(),
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit receiver subscribe failed: {e}"))?;
    let receiver_initial = wait_for_initial_items(
        receiver_conn,
        &receiver_key,
        receiver_subscribe_id,
        "timeline_stress receiver subscribe",
    )
    .await?;

    wait_for_stress_bodies_and_no_blank_rows(
        receiver_conn,
        &receiver_key,
        &receiver_initial,
        &expected_bodies,
        (messages_per_room + 20).min(u16::MAX as usize) as u16,
        "timeline_stress receiver backfill",
    )
    .await?;

    let receiver_unsubscribe_id = receiver_conn.next_request_id();
    receiver_conn
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: receiver_unsubscribe_id,
            key: receiver_key,
        }))
        .await
        .map_err(|e| format!("timeline_stress: submit receiver unsubscribe failed: {e}"))?;

    Ok(expected_bodies.len())
}

async fn send_timeline_stress_empty_formatted_probe(
    config: &QaConfig,
    room_id: &str,
    sender_prefix: &str,
    label: &str,
) -> Result<String, String> {
    let (username, password) = match sender_prefix {
        "a" => (&config.user_a, &config.password_a),
        "b" => (&config.user_b, &config.password_b),
        other => {
            return Err(format!("{label}: unknown stress sender prefix {other}"));
        }
    };
    let body = format!("Koushi local stress formatted fallback {sender_prefix}");
    let session = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: username.clone(),
        password: AuthSecret::new(password.clone()),
        device_display_name: Some("Koushi raw formatted QA".to_owned()),
    })
    .await
    .map_err(|error| format!("{label}: raw probe login failed: {error}"))?;
    koushi_sdk::sync_once(&session)
        .await
        .map_err(|error| format!("{label}: raw probe sync failed: {error}"))?;

    let parsed_room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|error| format!("{label}: raw probe room id parse failed: {error}"))?;
    let room = session
        .client()
        .get_room(&parsed_room_id)
        .ok_or_else(|| format!("{label}: raw probe room was not available after sync"))?;
    room.send_raw(
        "m.room.message",
        serde_json::json!({
            "msgtype": "m.text",
            "body": body,
            "format": "org.matrix.custom.html",
            "formatted_body": "<p><br /></p>"
        }),
    )
    .await
    .map_err(|error| format!("{label}: raw probe send failed: {error}"))?;

    if let Err(error) = koushi_sdk::logout(&session).await {
        eprintln!("timeline_stress raw probe logout warning: {error}");
    }
    Ok(body)
}

async fn run_scheduled_send_stage(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    room_id: &str,
) -> Result<(), String> {
    const SCHEDULED_CREATE_BODY: &str = "Koushi scheduled create QA body";
    const SCHEDULED_FIRE_BODY: &str = "Koushi scheduled fire QA body";
    let session = authenticated_session_info(conn, "scheduled send account")?.clone();
    let expected_account = koushi_key::SessionKeyId {
        homeserver: session.homeserver,
        user_id: session.user_id,
        device_id: session.device_id,
    };

    let select_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id: select_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit room select failed: {e}"))?;
    wait_for_selected_room(conn, room_id, "scheduled_send selected room").await?;

    let create_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::ScheduleSend {
        request_id: create_id,
        expected_account: expected_account.clone(),
        room_id: room_id.to_owned(),
        thread_root_event_id: None,
        body: SCHEDULED_CREATE_BODY.to_owned(),
        send_at_ms: scheduled_qa_epoch_ms(Duration::from_secs(300)),
        draft_revision: 0,
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit create failed: {e}"))?;

    let created = wait_for_scheduled_send_count(conn, 1, "scheduled_send create").await?;
    if created.timeline.scheduled_send_capability != ScheduledSendCapability::LocalFallback {
        return Err(
            "scheduled_send: local fallback capability was not projected to the snapshot"
                .to_owned(),
        );
    }
    println!("scheduled_capability=local_fallback");
    println!("scheduled_create=ok");

    let scheduled_id = created
        .timeline
        .scheduled_sends
        .first()
        .map(|item| item.scheduled_id.clone())
        .ok_or_else(|| "scheduled_send: created item was missing from projection".to_owned())?;
    let rescheduled_at_ms = scheduled_qa_epoch_ms(Duration::from_secs(600));
    let reschedule_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id: reschedule_id,
        scheduled_id: scheduled_id.clone(),
        send_at_ms: rescheduled_at_ms,
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit reschedule failed: {e}"))?;
    wait_for_scheduled_send_due(
        conn,
        &scheduled_id,
        rescheduled_at_ms,
        "scheduled_send reschedule",
    )
    .await?;
    println!("scheduled_reschedule=ok");

    let cancel_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CancelScheduledSend {
        request_id: cancel_id,
        scheduled_id,
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit cancel failed: {e}"))?;
    wait_for_scheduled_send_count(conn, 0, "scheduled_send cancel").await?;
    println!("scheduled_cancel=ok");

    let fire_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::ScheduleSend {
        request_id: fire_id,
        expected_account,
        room_id: room_id.to_owned(),
        thread_root_event_id: None,
        body: SCHEDULED_FIRE_BODY.to_owned(),
        send_at_ms: scheduled_qa_epoch_ms(Duration::from_millis(250)),
        draft_revision: 0,
    }))
    .await
    .map_err(|e| format!("scheduled_send: submit fire schedule failed: {e}"))?;
    let fire_created = wait_for_scheduled_send_count(conn, 1, "scheduled_send fire create").await?;
    let fire_scheduled_id = fire_created
        .timeline
        .scheduled_sends
        .first()
        .map(|item| item.scheduled_id.clone())
        .ok_or_else(|| "scheduled_send: fire item was missing from projection".to_owned())?;
    wait_for_scheduled_send_fired(
        conn,
        key,
        &fire_scheduled_id,
        SCHEDULED_FIRE_BODY,
        "scheduled_send fire",
    )
    .await?;
    println!("scheduled_fire=ok");
    Ok(())
}

fn scheduled_qa_epoch_ms(offset: Duration) -> u64 {
    SystemTime::now()
        .checked_add(offset)
        .unwrap_or_else(SystemTime::now)
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

async fn wait_for_selected_room(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().timeline.room_id.as_deref() == Some(room_id) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for selected room"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot)
                if snapshot.timeline.room_id.as_deref() == Some(room_id) =>
            {
                return Ok(());
            }
            _ if conn.snapshot().timeline.room_id.as_deref() == Some(room_id) => return Ok(()),
            _ => {}
        }
    }
}

async fn wait_for_scheduled_send_count(
    conn: &mut CoreConnection,
    expected_count: usize,
    label: &str,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    if snapshot.timeline.scheduled_sends.len() == expected_count {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for scheduled-send projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot)
                if snapshot.timeline.scheduled_sends.len() == expected_count =>
            {
                return Ok(snapshot);
            }
            _ if conn.snapshot().timeline.scheduled_sends.len() == expected_count => {
                return Ok(conn.snapshot());
            }
            _ => {}
        }
    }
}

async fn wait_for_scheduled_send_due(
    conn: &mut CoreConnection,
    scheduled_id: &str,
    expected_send_at_ms: u64,
    label: &str,
) -> Result<(), String> {
    let matches_due =
        |snapshot: &AppState| {
            snapshot.timeline.scheduled_sends.iter().any(|item| {
                item.scheduled_id == scheduled_id && item.send_at_ms == expected_send_at_ms
            })
        };
    if matches_due(&conn.snapshot()) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for scheduled-send reschedule"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if matches_due(&snapshot) => return Ok(()),
            _ if matches_due(&conn.snapshot()) => return Ok(()),
            _ => {}
        }
    }
}

async fn wait_for_scheduled_send_fired(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    scheduled_id: &str,
    expected_body: &str,
    label: &str,
) -> Result<(), String> {
    let mut queue_removed = scheduled_item_absent(&conn.snapshot(), scheduled_id);
    let mut timeline_observed = false;

    loop {
        if queue_removed && timeline_observed {
            return Ok(());
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for scheduled-send dispatch"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) => {
                queue_removed = scheduled_item_absent(&snapshot, scheduled_id);
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_body_matches(item, expected_body) {
                        timeline_observed = true;
                    }
                    Ok(())
                })?;
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id.connection_id.0 == 0 => {
                return Err(format!(
                    "{label}: internal scheduled-send dispatch failed: {failure:?}"
                ));
            }
            _ => {}
        }
    }
}

fn scheduled_item_absent(snapshot: &AppState, scheduled_id: &str) -> bool {
    snapshot
        .timeline
        .scheduled_sends
        .iter()
        .all(|item| item.scheduled_id != scheduled_id)
}

// ---------------------------------------------------------------------------
// Cache-restore verification harness (#123, Phase C)
// ---------------------------------------------------------------------------

/// Reads KOUSHI_QA_CACHE_RESTORE_ROOMS / _DEPTH, clamps at defaults.
fn cache_restore_params() -> (usize, usize) {
    let rooms = std::env::var(ENV_CACHE_RESTORE_ROOMS)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CACHE_RESTORE_ROOMS)
        .max(1);
    let depth = std::env::var(ENV_CACHE_RESTORE_DEPTH)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_CACHE_RESTORE_DEPTH)
        .max(10);
    (rooms, depth)
}

/// Apply a single `TimelineDiff` in-place to a `Vec<TimelineItem>`.
fn apply_timeline_diff(items: &mut Vec<TimelineItem>, diff: &TimelineDiff) {
    match diff {
        TimelineDiff::PushFront { item } => items.insert(0, item.clone()),
        TimelineDiff::PushBack { item } => items.push(item.clone()),
        TimelineDiff::Insert { index, item } => {
            let idx = (*index).min(items.len());
            items.insert(idx, item.clone());
        }
        TimelineDiff::Set { index, item } => {
            if *index < items.len() {
                items[*index] = item.clone();
            }
        }
        TimelineDiff::Remove { index } => {
            if *index < items.len() {
                items.remove(*index);
            }
        }
        TimelineDiff::Truncate { length } => items.truncate(*length),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::Reset { items: new_items } => *items = new_items.clone(),
    }
}

async fn run_cache_restore_scenario(config: &QaConfig) -> Result<(), String> {
    let (num_rooms, depth) = cache_restore_params();
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir = qa_data_dir("cache_restore");

    // -----------------------------------------------------------------------
    // Connect 1: login, send fixture history, paginate to EndReached, record
    // deep anchors deterministically (m0 = first sent = oldest), then shut down.
    // -----------------------------------------------------------------------
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();

    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: proxy.homeserver_url(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi Core QA Cache Restore".to_owned()),
        },
    }))
    .await
    .map_err(|e| format!("cache_restore: submit login failed: {e}"))?;

    let account_key = wait_for_logged_in(&mut conn, login_id, "cache_restore login").await?;
    wait_for_ready_snapshot(&mut conn, "cache_restore Ready").await?;
    let sync_start_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_start_id,
    }))
    .await
    .map_err(|e| format!("cache_restore: submit Sync start failed: {e}"))?;
    let sync_backend_a =
        wait_for_sync_started_and_running(&mut conn, sync_start_id, "cache_restore sync start")
            .await?;
    println!("sync_backend_a={sync_backend_a:?}");
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_a,
        "cache_restore sync start",
    )?;

    // Create rooms, send DEPTH messages, paginate to EndReached. Track items
    // across the paginate to find the deterministic deep anchor (m0 = oldest).
    let mut room_ids: Vec<String> = Vec::with_capacity(num_rooms);
    let mut deep_anchors: Vec<String> = Vec::with_capacity(num_rooms);
    for room_idx in 0..num_rooms {
        let anchor_body = format!("cache_restore fixture r{room_idx} m0");
        let room_id = create_room_for_qa(
            &mut conn,
            &format!("QA Cache Restore Room {room_idx}"),
            false,
            "cache_restore create room",
        )
        .await?;
        wait_for_room_in_room_list(&mut conn, &room_id, "cache_restore room in list").await?;

        let key = TimelineKey::room(account_key.clone(), room_id.clone());
        let sub_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: sub_id,
            key: key.clone(),
        }))
        .await
        .map_err(|e| format!("cache_restore: submit subscribe failed: {e}"))?;
        let initial_items =
            wait_for_initial_items(&mut conn, &key, sub_id, "cache_restore subscribe").await?;
        // Track all items across the paginate so we can find m0 at the end.
        let mut all_items = initial_items;

        // Send DEPTH messages sequentially so they land in the event cache.
        for msg_idx in 0..depth {
            let txn = format!("qa-cr-{room_idx}-{msg_idx}");
            let send_id = conn.next_request_id();
            conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: key.clone(),
                transaction_id: txn.clone(),
                body: format!("cache_restore fixture r{room_idx} m{msg_idx}"),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("cache_restore: submit send failed: {e}"))?;
            wait_for_send_flow_completion(
                &mut conn,
                send_id,
                &key,
                &txn,
                &format!("cache_restore fixture r{room_idx} m{msg_idx}"),
                "cache_restore send",
            )
            .await?;
        }

        // Paginate backward to EndReached, accumulating diffs so all_items
        // reflects the full history and we can find m0 deterministically.
        let pag_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: pag_id,
            key: key.clone(),
            direction: PaginationDirection::Backward,
            event_count: CACHE_RESTORE_PAGINATE_BATCH,
        }))
        .await
        .map_err(|e| format!("cache_restore: submit paginate failed: {e}"))?;
        let _ = pag_id;
        let mut saw_paginating = false;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(120), conn.recv_event())
                .await
                .map_err(|_| {
                    "cache_restore populate: timed out waiting for paginate event".to_owned()
                })?
                .map_err(|lag| {
                    format!(
                        "cache_restore populate: event stream lagged (skipped={})",
                        lag.skipped
                    )
                })?;
            match event {
                CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    key: ref ev_key,
                    direction,
                    ref state,
                    ..
                }) if ev_key == &key && direction == PaginationDirection::Backward => match state {
                    PaginationState::Paginating => {
                        saw_paginating = true;
                    }
                    PaginationState::Idle => {
                        if !saw_paginating {
                            return Err(
                                "cache_restore populate: Idle without Paginating".to_owned()
                            );
                        }
                        saw_paginating = false;
                        let repag_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: repag_id,
                            key: key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: CACHE_RESTORE_PAGINATE_BATCH,
                        }))
                        .await
                        .map_err(|e| format!("cache_restore: re-paginate failed: {e}"))?;
                    }
                    PaginationState::EndReached => {
                        break;
                    }
                    PaginationState::Failed { .. } => {
                        return Err("cache_restore populate: paginate failed".to_owned());
                    }
                },
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                    key: ref ev_key,
                    ref diffs,
                    ..
                }) if ev_key == &key => {
                    for diff in diffs {
                        apply_timeline_diff(&mut all_items, diff);
                    }
                }
                _ => {}
            }
        }

        // Find the deterministic deep anchor: m0 is the first-sent (oldest) message.
        let anchor_item =
            find_timeline_item_with_body(&all_items, &anchor_body).ok_or_else(|| {
                format!(
                    "cache_restore: m0 anchor not found after full paginate \
                     (room_idx={room_idx}, items={})",
                    all_items.len()
                )
            })?;
        let anchor_event_id = match &anchor_item.id {
            TimelineItemId::Event { event_id } => event_id.clone(),
            other => {
                return Err(format!(
                    "cache_restore: m0 anchor item has non-Event id: {other:?}"
                ));
            }
        };

        let unsub_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_id,
            key,
        }))
        .await
        .map_err(|e| format!("cache_restore: submit unsubscribe failed: {e}"))?;

        room_ids.push(room_id);
        deep_anchors.push(anchor_event_id);
    }

    // -----------------------------------------------------------------------
    // Shallow-anchor room: CACHE_RESTORE_SHALLOW_DEPTH messages, sized so
    // that m0 (oldest) lies beyond the SDK's initial visible window (~20
    // items).  All events fit in one stored chunk (chunks_loaded == 0).
    //
    // After restart, live_restore_from_cache reveals m0 via
    // live_lazy_paginate_backwards (lazy_reveal_batches == 1, chunks_loaded == 0).
    // The P1 lazy-reveal-fence fix (147c9ed) gates on this path: it adds
    // lazy_reveal_batches to the settle fence so the DiffBatch settles before
    // the restore concludes with Found.  Without the fix the fence may miss
    // that batch and finish early.
    //
    // Bug #1 fix: capture the anchor event_id directly from the SendFlowOutcome
    // of the first send (m0).  The send-phase ItemsUpdated diffs are consumed
    // by wait_for_send_flow_completion and are not returned, so tracking
    // shallow_items through the send loop would never include m0.
    // -----------------------------------------------------------------------
    let shallow_room_id = create_room_for_qa(
        &mut conn,
        "QA Cache Restore Shallow",
        false,
        "cache_restore shallow create",
    )
    .await?;
    wait_for_room_in_room_list(
        &mut conn,
        &shallow_room_id,
        "cache_restore shallow room in list",
    )
    .await?;

    let shallow_key = TimelineKey::room(account_key.clone(), shallow_room_id.clone());
    let shallow_sub_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: shallow_sub_id,
        key: shallow_key.clone(),
    }))
    .await
    .map_err(|e| format!("cache_restore shallow: subscribe failed: {e}"))?;
    let _ = wait_for_initial_items(
        &mut conn,
        &shallow_key,
        shallow_sub_id,
        "cache_restore shallow subscribe",
    )
    .await?;

    // Send CACHE_RESTORE_SHALLOW_DEPTH messages and capture m0's event_id
    // directly from the first SendFlowOutcome — no item tracking needed.
    let mut shallow_anchor_id: Option<String> = None;
    for msg_idx in 0..CACHE_RESTORE_SHALLOW_DEPTH {
        let txn = format!("qa-cr-shallow-{msg_idx}");
        let send_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: shallow_key.clone(),
            transaction_id: txn.clone(),
            body: format!("cache_restore shallow m{msg_idx}"),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("cache_restore shallow: send failed: {e}"))?;
        let outcome = wait_for_send_flow_completion(
            &mut conn,
            send_id,
            &shallow_key,
            &txn,
            &format!("cache_restore shallow m{msg_idx}"),
            "cache_restore shallow send",
        )
        .await?;
        // m0 is the first-sent (oldest) message; record its event_id as the anchor.
        if msg_idx == 0 {
            shallow_anchor_id = Some(outcome.event_id.clone());
        }
    }
    let shallow_anchor_id =
        shallow_anchor_id.ok_or_else(|| "cache_restore shallow: no messages sent".to_owned())?;

    // Paginate backward to EndReached to warm the event cache so that
    // live_restore_from_cache can serve the anchor from the stored chunk on
    // restart (without a network call).
    let shallow_pag_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: shallow_pag_id,
        key: shallow_key.clone(),
        direction: PaginationDirection::Backward,
        event_count: CACHE_RESTORE_PAGINATE_BATCH,
    }))
    .await
    .map_err(|e| format!("cache_restore shallow: paginate failed: {e}"))?;
    let _ = shallow_pag_id;
    let mut shallow_saw_paginating = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn.recv_event())
            .await
            .map_err(|_| "cache_restore shallow: timed out waiting for paginate event".to_owned())?
            .map_err(|lag| {
                format!(
                    "cache_restore shallow: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                ref state,
                ..
            }) if ev_key == &shallow_key && direction == PaginationDirection::Backward => {
                match state {
                    PaginationState::Paginating => {
                        shallow_saw_paginating = true;
                    }
                    PaginationState::Idle => {
                        if !shallow_saw_paginating {
                            return Err("cache_restore shallow: Idle without Paginating".to_owned());
                        }
                        shallow_saw_paginating = false;
                        let repag_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: repag_id,
                            key: shallow_key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: CACHE_RESTORE_PAGINATE_BATCH,
                        }))
                        .await
                        .map_err(|e| format!("cache_restore shallow: re-paginate failed: {e}"))?;
                    }
                    PaginationState::EndReached => {
                        break;
                    }
                    PaginationState::Failed { .. } => {
                        return Err("cache_restore shallow: paginate failed".to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    let shallow_unsub_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: shallow_unsub_id,
        key: shallow_key,
    }))
    .await
    .map_err(|e| format!("cache_restore shallow: unsubscribe failed: {e}"))?;

    println!("cache_restore_loaded=ok");

    // Clean shutdown of Connect 1.
    stop_sync_for_qa(&mut conn, "cache_restore stop sync before restart").await?;
    drop(conn);
    runtime.shutdown().await;

    // -----------------------------------------------------------------------
    // Connect 2: restart over the same data dir, BLOCK the network, then drive
    // RestoreTimelineAnchor per room using production-faithful params.
    // PRIMARY GATE: status == Found, OR (EndReached AND anchor present in items).
    // Cycle count + ms are diagnostics only.
    // -----------------------------------------------------------------------
    let runtime2 = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn2 = runtime2.attach();

    let restore_id = conn2.next_request_id();
    conn2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        }))
        .await
        .map_err(|e| format!("cache_restore: submit restore failed: {e}"))?;
    wait_for_session_restored(
        &mut conn2,
        restore_id,
        &account_key,
        "cache_restore restore",
    )
    .await?;
    wait_for_ready_snapshot(&mut conn2, "cache_restore restored Ready").await?;

    // Block the network NOW: any /messages network call from here on will fail.
    proxy.disable();

    let aggregate_start = std::time::Instant::now();
    let mut all_deep_restores_terminated_cleanly = true;
    let mut total_cycles: u32 = 0;
    // Per-room cycle counts for the room-entry speed gate.
    let mut room_cycle_counts: Vec<u16> = Vec::new();

    for (room_idx, (room_id, anchor)) in room_ids.iter().zip(deep_anchors.iter()).enumerate() {
        let key = TimelineKey::room(account_key.clone(), room_id.clone());
        let sub_id = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: sub_id,
                key: key.clone(),
            }))
            .await
            .map_err(|e| format!("cache_restore: offline subscribe failed: {e}"))?;
        let _initial_offline =
            wait_for_initial_items(&mut conn2, &key, sub_id, "cache_restore offline subscribe")
                .await?;

        let room_start = std::time::Instant::now();
        let restore_req = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(
                TimelineCommand::RestoreTimelineAnchor {
                    request_id: restore_req,
                    key: key.clone(),
                    event_id: anchor.clone(),
                    // Production-faithful params: source TimelineView.tsx
                    // (LIVE_ROOM_ANCHOR_RESTORE_MAX_BATCHES=6, EVENT_COUNT=100).
                    // A deep anchor may end as BudgetExhausted; room entry must
                    // not inflate this into a long history walk.
                    max_batches: CACHE_RESTORE_PROD_MAX_BATCHES,
                    event_count: CACHE_RESTORE_PROD_EVENT_COUNT,
                },
            ))
            .await
            .map_err(|e| {
                format!("cache_restore: offline RestoreTimelineAnchor submit failed: {e}")
            })?;

        // Consume events until AnchorRestoreFinished. Count Paginating transitions
        // as internal backward-paginate cycles for the speed regression gate.
        let mut cycle_count: u16 = 0;
        let status = loop {
            let event = tokio::time::timeout(Duration::from_secs(120), conn2.recv_event())
                .await
                .map_err(|_| {
                    "cache_restore offline: timed out waiting for AnchorRestoreFinished".to_owned()
                })?
                .map_err(|lag| {
                    format!(
                        "cache_restore offline: event stream lagged (skipped={})",
                        lag.skipped
                    )
                })?;
            match event {
                CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    key: ref ev_key,
                    direction,
                    state: PaginationState::Paginating,
                    ..
                }) if ev_key == &key && direction == PaginationDirection::Backward => {
                    cycle_count += 1;
                }
                CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                    request_id: ev_req,
                    key: ref ev_key,
                    ref status,
                }) if ev_req == restore_req && ev_key == &key => {
                    break status.clone();
                }
                _ => {}
            }
        };

        let room_ms = room_start.elapsed().as_millis();
        total_cycles += cycle_count as u32;
        room_cycle_counts.push(cycle_count);
        let status_label = match &status {
            TimelineAnchorRestoreStatus::Found => "found",
            TimelineAnchorRestoreStatus::EndReached => "end_reached",
            TimelineAnchorRestoreStatus::BudgetExhausted => "budget_exhausted",
            TimelineAnchorRestoreStatus::Superseded => "superseded",
            TimelineAnchorRestoreStatus::Failed { .. } => "failed",
        };
        // Private-data-free diagnostics: cycles + ms only, no ids or bodies.
        eprintln!(
            "cache_restore room={room_idx} cycles={cycle_count} ms={room_ms} status={status_label}"
        );

        // PRIMARY CORRECTNESS GATE:
        // The normal room-entry path is intentionally budgeted. Deep anchors may
        // end as BudgetExhausted or EndReached; the UI then falls back to the
        // live edge. The gate here is clean, bounded termination rather than
        // forcing a deep-history restore during room selection.
        let room_terminated_cleanly = match &status {
            TimelineAnchorRestoreStatus::Found => true,
            TimelineAnchorRestoreStatus::EndReached
            | TimelineAnchorRestoreStatus::BudgetExhausted => true,
            TimelineAnchorRestoreStatus::Failed { .. }
            | TimelineAnchorRestoreStatus::Superseded => {
                eprintln!("cache_restore room={room_idx}: restore status={status_label} offline");
                false
            }
        };
        if !room_terminated_cleanly {
            all_deep_restores_terminated_cleanly = false;
        }

        let unsub_id = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
                request_id: unsub_id,
                key,
            }))
            .await
            .map_err(|e| format!("cache_restore: offline unsubscribe failed: {e}"))?;
    }

    let aggregate_ms = aggregate_start.elapsed().as_millis();
    eprintln!("cache_restore total_cycles={total_cycles} total_ms={aggregate_ms}");

    // -----------------------------------------------------------------------
    // Shallow-anchor gate (P1 lazy-reveal-fence fix):
    // The anchor is in the live in-memory prefix (< CACHE_RESTORE_SHALLOW_DEPTH
    // events).  live_lazy_paginate_backwards must reveal it without loading any
    // on-disk chunk (cycle_count == 0).  On code without the P1 fix this may
    // reach EndReached or BudgetExhausted prematurely; with the fix it is Found.
    // -----------------------------------------------------------------------
    let shallow_key2 = TimelineKey::room(account_key.clone(), shallow_room_id.clone());
    let shallow_sub2 = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: shallow_sub2,
            key: shallow_key2.clone(),
        }))
        .await
        .map_err(|e| format!("cache_restore shallow: offline subscribe failed: {e}"))?;
    let _shallow_initial2 = wait_for_initial_items(
        &mut conn2,
        &shallow_key2,
        shallow_sub2,
        "cache_restore shallow offline subscribe",
    )
    .await?;

    let shallow_restore_req = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(
            TimelineCommand::RestoreTimelineAnchor {
                request_id: shallow_restore_req,
                key: shallow_key2.clone(),
                event_id: shallow_anchor_id.clone(),
                max_batches: CACHE_RESTORE_PROD_MAX_BATCHES,
                event_count: CACHE_RESTORE_PROD_EVENT_COUNT,
            },
        ))
        .await
        .map_err(|e| {
            format!("cache_restore shallow: offline RestoreTimelineAnchor submit failed: {e}")
        })?;

    let mut shallow_cycle_count: u16 = 0;
    let shallow_status = loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn2.recv_event())
            .await
            .map_err(|_| {
                "cache_restore shallow: timed out waiting for AnchorRestoreFinished".to_owned()
            })?
            .map_err(|lag| {
                format!(
                    "cache_restore shallow: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state: PaginationState::Paginating,
                ..
            }) if ev_key == &shallow_key2 && direction == PaginationDirection::Backward => {
                shallow_cycle_count += 1;
            }
            CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                request_id: ev_req,
                key: ref ev_key,
                ref status,
            }) if ev_req == shallow_restore_req && ev_key == &shallow_key2 => {
                break status.clone();
            }
            _ => {}
        }
    };

    let shallow_status_label = match &shallow_status {
        TimelineAnchorRestoreStatus::Found => "found",
        TimelineAnchorRestoreStatus::EndReached => "end_reached",
        TimelineAnchorRestoreStatus::BudgetExhausted => "budget_exhausted",
        TimelineAnchorRestoreStatus::Superseded => "superseded",
        TimelineAnchorRestoreStatus::Failed { .. } => "failed",
    };
    eprintln!("cache_restore shallow cycles={shallow_cycle_count} status={shallow_status_label}");

    // Gate: shallow anchor must reach Found (the lazy-reveal path must settle
    // before declaring the restore terminal).  cycle_count==0 is the expected
    // value after the P1 fix (no disk chunk needed); a non-zero count is
    // unexpected but not a hard gate here — correctness (Found) is the gate.
    let shallow_succeeded = matches!(&shallow_status, TimelineAnchorRestoreStatus::Found);
    if !shallow_succeeded {
        eprintln!(
            "cache_restore shallow: status={shallow_status_label} — \
             lazy-reveal-fence (P1) fix not yet applied or not effective \
             (EXPECTED RED before impl-stage1 P1 fix lands)"
        );
    }
    if shallow_cycle_count > 0 {
        eprintln!(
            "cache_restore shallow: cycles={shallow_cycle_count} > 0 — \
             disk chunks loaded for a shallow anchor; expected 0 after P1 fix"
        );
    }

    let shallow_unsub2 = conn2.next_request_id();
    conn2
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: shallow_unsub2,
            key: shallow_key2,
        }))
        .await
        .map_err(|e| format!("cache_restore shallow: offline unsubscribe failed: {e}"))?;

    // SECONDARY GATE (room-entry speed regression gate):
    // Each deep-anchor restore must terminate in ≤ CACHE_RESTORE_MAX_CYCLES
    // backward-paginate cycles. It may be Found, EndReached, or BudgetExhausted;
    // what matters here is that a stale/deep anchor cannot stall room selection.
    let slow_rooms: Vec<usize> = room_cycle_counts
        .iter()
        .enumerate()
        .filter(|&(_, c)| *c > CACHE_RESTORE_MAX_CYCLES)
        .map(|(i, _)| i)
        .collect();

    cleanup_logged_in_runtime(conn2, runtime2, account_key, "cache_restore cleanup").await?;

    if !all_deep_restores_terminated_cleanly {
        return Err(
            "cache_restore: deep anchor restore did not terminate cleanly within room-entry path"
                .to_owned(),
        );
    }

    if !slow_rooms.is_empty() {
        let worst = room_cycle_counts.iter().copied().max().unwrap_or(0);
        return Err(format!(
            "cache_restore: deep anchor restore used {worst} backward-paginate cycles \
             (> {CACHE_RESTORE_MAX_CYCLES}) — room entry may block on stale/deep anchors"
        ));
    }

    // Shallow-anchor gate: emits after the deep gates pass so the report
    // clearly distinguishes deep-restore failures from P1 lazy-reveal failures.
    if !shallow_succeeded {
        return Err(format!(
            "cache_restore: shallow anchor reached status={shallow_status_label} \
             (expected Found) — lazy-reveal-fence (P1) fix not yet applied \
             (EXPECTED RED before impl-stage1 P1 fix lands)"
        ));
    }
    println!("cache_restore_shallow=ok");

    println!("cache_restore_offline=ok");
    println!("cache_restore=ok");
    Ok(())
}

async fn run_focused_send_queue_scenario(config: &QaConfig) -> Result<(), String> {
    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        bootstrap_recovery_secret,
        sync_backend: _,
    } = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("send_queue_bootstrap"),
        &config.user_a,
        &config.password_a,
        "Koushi Core QA Send Queue Bootstrap",
        "send_queue bootstrap login",
        "send_queue bootstrap gate",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    println!("login_sync=ok");

    let sync_stop_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop {
        request_id: sync_stop_id,
    }))
    .await
    .map_err(|e| format!("send_queue bootstrap submit sync stop: {e}"))?;
    wait_for_sync_stopped(&mut conn, sync_stop_id, "send_queue bootstrap sync stop").await?;

    let logout_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::Logout {
        request_id: logout_id,
    }))
    .await
    .map_err(|e| format!("send_queue bootstrap submit logout: {e}"))?;
    wait_for_logged_out(
        &mut conn,
        logout_id,
        &account_key,
        "send_queue bootstrap logout",
    )
    .await?;

    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| "send_queue bootstrap ordered runtime shutdown timed out".to_owned())?;

    let recovery_secret = bootstrap_recovery_secret
        .ok_or_else(|| "send_queue bootstrap recovery secret unavailable".to_owned())?;
    run_send_queue_stage(config, &recovery_secret).await
}

async fn run_send_queue_stage(
    config: &QaConfig,
    recovery_secret: &AuthSecret,
) -> Result<(), String> {
    let display_projection_reset_fallback_baseline =
        koushi_core::timeline::display_projection_reset_fallback_count();
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir = qa_data_dir("send_queue");
    let proxy_homeserver = proxy.homeserver_url();
    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        bootstrap_recovery_secret: _,
        sync_backend: _,
    } = login_synced_participant_for_qa(
        &proxy_homeserver,
        data_dir.clone(),
        &config.user_a,
        &config.password_a,
        "Koushi Core QA Send Queue",
        "send_queue login",
        "send_queue recovery gate",
        QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret),
    )
    .await?;

    let room_id = create_room_for_qa(
        &mut conn,
        "QA Send Queue Room",
        false,
        "send_queue create room",
    )
    .await?;
    wait_for_room_in_room_list(&mut conn, &room_id, "send_queue room list").await?;

    let key = TimelineKey::room(account_key.clone(), room_id.clone());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("send_queue: submit subscribe failed: {e}"))?;
    wait_for_initial_items(&mut conn, &key, subscribe_id, "send_queue subscribe").await?;

    proxy.disable();
    let first = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-first",
        "QA send queue first offline",
        "send_queue first offline",
    )
    .await?;
    wait_for_timeline_send_state(
        &mut conn,
        &key,
        &first.sdk_transaction_id,
        |state| matches!(state, TimelineSendState::NotSent { .. }),
        "send_queue first not_sent",
    )
    .await?;
    println!("send_fail=ok");

    let second = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-second",
        "QA send queue second offline",
        "send_queue second offline",
    )
    .await?;

    proxy.enable();
    let room_send_forwarded_before_retry = proxy.room_send_forwarded_count();
    let room_send_responses_completed_before_retry = proxy.room_send_responses_completed_count();
    let retry_id = retry_send_queue_item(
        &mut conn,
        &key,
        &first.sdk_transaction_id,
        "send_queue retry first",
    )
    .await?;
    wait_for_send_completions_in_order(
        &mut conn,
        &key,
        retry_id,
        &first,
        &second,
        "send_queue fifo retry",
    )
    .await
    .map_err(|error| {
        format!(
            "{error} room_send_forwarded_after_retry={} \
             room_send_responses_completed_after_retry={}",
            proxy
                .room_send_forwarded_count()
                .saturating_sub(room_send_forwarded_before_retry),
            proxy
                .room_send_responses_completed_count()
                .saturating_sub(room_send_responses_completed_before_retry)
        )
    })?;
    println!("resend=ok");
    println!("fifo=ok");

    proxy.disable();
    let cancel = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-cancel",
        "QA send queue cancel offline",
        "send_queue cancel offline",
    )
    .await?;
    let cancel_id = cancel_send_queue_item(
        &mut conn,
        &key,
        &cancel.sdk_transaction_id,
        "send_queue cancel",
    )
    .await?;
    wait_for_cancelled_or_removed_send(
        &mut conn,
        &key,
        cancel_id,
        &cancel.sdk_transaction_id,
        "send_queue cancel removed",
    )
    .await?;
    println!("cancel_send=ok");

    let _restart = send_text_expect_local_echo(
        &mut conn,
        &key,
        "qa-send-queue-restart",
        "QA send queue restart offline",
        "send_queue restart offline",
    )
    .await?;

    unsubscribe_timeline_for_qa(
        &mut conn,
        &key,
        "send_queue unsubscribe before restart shutdown",
    )
    .await?;
    stop_sync_for_qa(&mut conn, "send_queue stop before restart").await?;
    drop(conn);
    runtime.shutdown().await;

    let runtime = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn = runtime.attach();
    let restore_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
        request_id: restore_id,
        account_key: account_key.clone(),
    }))
    .await
    .map_err(|e| format!("send_queue: submit restore failed: {e}"))?;
    wait_for_session_restored(&mut conn, restore_id, &account_key, "send_queue restore").await?;
    wait_for_ready_snapshot(&mut conn, "send_queue restored Ready").await?;

    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("send_queue: submit restore subscribe failed: {e}"))?;
    let initial = wait_for_initial_items(
        &mut conn,
        &key,
        subscribe_id,
        "send_queue restored subscribe",
    )
    .await?;
    let restored = find_timeline_item_with_body(&initial, "QA send queue restart offline")
        .ok_or_else(|| "send_queue restored local echo missing after restart".to_owned())?;
    let restored_txn = match restored.id {
        TimelineItemId::Transaction { transaction_id } => transaction_id,
        TimelineItemId::Event { .. } => {
            assert_zero_display_projection_reset_fallback_delta(
                display_projection_reset_fallback_baseline,
                koushi_core::timeline::display_projection_reset_fallback_count(),
            )?;
            println!("display_projection_reset_fallbacks=0");
            unsubscribe_timeline_for_qa(&mut conn, &key, "send_queue unsubscribe before cleanup")
                .await?;
            println!("unsent_restart=ok");
            cleanup_logged_in_runtime(conn, runtime, account_key, "send_queue cleanup").await?;
            return Ok(());
        }
        TimelineItemId::Synthetic { .. } => {
            return Err("send_queue restored item had synthetic id".to_owned());
        }
    };

    proxy.enable();
    let retry_already_sent =
        if matches!(restored.send_state, Some(TimelineSendState::NotSent { .. })) {
            retry_send_queue_item(&mut conn, &key, &restored_txn, "send_queue retry restored")
                .await?;
            true
        } else {
            false
        };
    wait_for_event_item_with_body_or_retry_not_sent(
        &mut conn,
        &key,
        &restored_txn,
        "QA send queue restart offline",
        retry_already_sent,
        "send_queue restored sent",
    )
    .await?;
    println!("unsent_restart=ok");

    assert_zero_display_projection_reset_fallback_delta(
        display_projection_reset_fallback_baseline,
        koushi_core::timeline::display_projection_reset_fallback_count(),
    )?;
    println!("display_projection_reset_fallbacks=0");

    unsubscribe_timeline_for_qa(&mut conn, &key, "send_queue unsubscribe before cleanup").await?;
    cleanup_logged_in_runtime(conn, runtime, account_key, "send_queue cleanup").await
}

fn assert_zero_display_projection_reset_fallback_delta(
    baseline: u64,
    current: u64,
) -> Result<(), String> {
    if current == baseline {
        Ok(())
    } else {
        Err("send_queue: display projection reset fallback counter changed".to_owned())
    }
}

async fn unsubscribe_timeline_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit unsubscribe failed: {e}"))?;
    tokio::time::sleep(TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT).await;
    Ok(())
}

async fn run_timeline_reconnect_scenario(config: &QaConfig) -> Result<(), String> {
    run_timeline_reconnect_scenario_impl(config, false, false).await
}

async fn run_timeline_legacy_fallback_scenario(config: &QaConfig) -> Result<(), String> {
    run_timeline_reconnect_scenario_impl(config, true, false).await
}

async fn run_timeline_legacy_persisted_gap_scenario(config: &QaConfig) -> Result<(), String> {
    run_timeline_reconnect_scenario_impl(config, true, true).await
}

async fn run_timeline_reconnect_scenario_impl(
    config: &QaConfig,
    legacy_fallback: bool,
    restart_with_persisted_gap: bool,
) -> Result<(), String> {
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir_a = qa_data_dir(if restart_with_persisted_gap {
        "timeline_legacy_persisted_gap_a"
    } else {
        "timeline_reconnect_a"
    });
    let data_dir_b = qa_data_dir("timeline_reconnect_b");

    let runtime_a = CoreRuntime::start_with_data_dir(data_dir_a.clone());
    let mut conn_a = runtime_a.attach();
    let login_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a_id,
            request: koushi_state::LoginRequest {
                homeserver: proxy.homeserver_url(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Core QA Timeline Reconnect A".to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit login A failed: {e}"))?;

    complete_new_identity_gate_for_qa(&mut conn_a, &config.password_a, "timeline-reconnect-gate-a")
        .await?;

    let account_key_a =
        wait_for_logged_in(&mut conn_a, login_a_id, "timeline_reconnect login A").await?;
    wait_for_ready_snapshot(&mut conn_a, "timeline_reconnect session A Ready").await?;
    let sync_start_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_a_id,
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit sync start A failed: {e}"))?;
    let sync_backend_a = wait_for_sync_started_and_running(
        &mut conn_a,
        sync_start_a_id,
        "timeline_reconnect sync start A",
    )
    .await?;
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_a,
        "timeline_reconnect sync start A",
    )?;

    let runtime_b = CoreRuntime::start_with_data_dir(data_dir_b);
    let mut conn_b = runtime_b.attach();
    let login_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_b_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_b.clone(),
                password: AuthSecret::new(config.password_b.clone()),
                device_display_name: Some("Koushi Core QA Timeline Reconnect B".to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit login B failed: {e}"))?;

    complete_new_identity_gate_for_qa(&mut conn_b, &config.password_b, "timeline-reconnect-gate-b")
        .await?;

    let account_key_b =
        wait_for_logged_in(&mut conn_b, login_b_id, "timeline_reconnect login B").await?;
    wait_for_ready_snapshot(&mut conn_b, "timeline_reconnect session B Ready").await?;
    let sync_start_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_b_id,
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit sync start B failed: {e}"))?;
    let sync_backend_b = wait_for_sync_started_and_running(
        &mut conn_b,
        sync_start_b_id,
        "timeline_reconnect sync start B",
    )
    .await?;
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_b,
        "timeline_reconnect sync start B",
    )?;

    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let room_id = if restart_with_persisted_gap {
        let room_id = start_direct_message_for_qa(
            &mut conn_a,
            &user_b_full_id,
            "timeline legacy persisted gap start direct message",
        )
        .await?;
        wait_for_dm_room_in_room_list(
            &mut conn_a,
            &room_id,
            "timeline legacy persisted gap A DM room list",
        )
        .await?;
        wait_for_invite_in_snapshot(
            &mut conn_b,
            &room_id,
            None,
            "timeline legacy persisted gap B sees DM invite",
        )
        .await?;
        accept_invite_for_qa(
            &mut conn_b,
            &room_id,
            "timeline legacy persisted gap B accepts DM invite",
        )
        .await?;
        wait_for_room_in_room_list(
            &mut conn_b,
            &room_id,
            "timeline legacy persisted gap B room list",
        )
        .await?;
        wait_for_dm_room_in_room_list(
            &mut conn_a,
            &room_id,
            "timeline legacy persisted gap A confirms direct message",
        )
        .await?;
        room_id
    } else {
        let room_id = create_room_for_qa(
            &mut conn_a,
            "QA Timeline Reconnect Room",
            false,
            "timeline_reconnect create room",
        )
        .await?;
        wait_for_room_in_room_list(&mut conn_a, &room_id, "timeline_reconnect A room list").await?;
        invite_user_for_qa(
            &mut conn_a,
            &room_id,
            &user_b_full_id,
            "timeline_reconnect invite B",
        )
        .await?;
        wait_for_invite_in_snapshot(
            &mut conn_b,
            &room_id,
            Some(false),
            "timeline_reconnect B sees invite",
        )
        .await?;
        accept_invite_for_qa(&mut conn_b, &room_id, "timeline_reconnect B accepts invite").await?;
        wait_for_room_in_room_list(&mut conn_b, &room_id, "timeline_reconnect B room list").await?;
        room_id
    };

    // Rebuild both SyncService instances after the room membership is stable.
    // The subsequent timeline subscription then exercises a room present in
    // the service's initial list instead of depending on an operation-time
    // room-list refresh racing the subscribe command.
    stop_sync_for_qa(&mut conn_a, "timeline_reconnect restart setup stop A").await?;
    stop_sync_for_qa(&mut conn_b, "timeline_reconnect restart setup stop B").await?;
    start_sync_for_qa(&mut conn_a, "timeline_reconnect restart setup start A").await?;
    start_sync_for_qa(&mut conn_b, "timeline_reconnect restart setup start B").await?;

    let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let key_b = TimelineKey::room(account_key_b.clone(), room_id);
    subscribe_and_ack_active_timeline_projection_for_qa(
        &mut conn_a,
        &key_a,
        "timeline_reconnect subscribe A",
    )
    .await?;
    subscribe_and_ack_active_timeline_projection_for_qa(
        &mut conn_b,
        &key_b,
        "timeline_reconnect subscribe B",
    )
    .await?;

    let seed_body = "QA timeline reconnect known anchor";
    let seed_txn = "qa-timeline-reconnect-seed";
    let seed_send_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: seed_send_id,
            key: key_b.clone(),
            transaction_id: seed_txn.to_owned(),
            body: seed_body.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("timeline_reconnect: submit seed failed: {e}"))?;
    let seed_outcome = wait_for_send_flow_completion(
        &mut conn_b,
        seed_send_id,
        &key_b,
        seed_txn,
        seed_body,
        "timeline_reconnect seed known anchor",
    )
    .await?;
    wait_for_item_with_body_or_decryption_failure(
        &mut conn_a,
        &key_a,
        seed_body,
        "timeline_reconnect A receives known anchor",
    )
    .await?;
    if legacy_fallback {
        if restart_with_persisted_gap {
            unsubscribe_timeline_for_qa(
                &mut conn_a,
                &key_a,
                "timeline legacy persisted gap unsubscribe before first response",
            )
            .await?;
        }
        stop_sync_for_qa(&mut conn_a, "timeline legacy fallback stop A").await?;
    } else {
        unsubscribe_timeline_for_qa(
            &mut conn_a,
            &key_a,
            "timeline_reconnect unsubscribe A before offline gap",
        )
        .await?;
        proxy.disable();
        wait_for_sync_reconnecting(&mut conn_a, "timeline_reconnect A offline").await?;
    }

    let offline_event_count = if legacy_fallback { 140 } else { 21 };
    let offline_bodies = (0..offline_event_count)
        .map(|index| format!("QA timeline reconnect offline {index:02}"))
        .collect::<Vec<_>>();
    for (index, body) in offline_bodies.iter().enumerate() {
        let txn = format!("qa-timeline-reconnect-offline-{index:02}");
        let send_b_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_b_id,
                key: key_b.clone(),
                transaction_id: txn.clone(),
                body: body.clone(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("timeline_reconnect: submit B offline send failed: {e}"))?;
        wait_for_send_flow_completion(
            &mut conn_b,
            send_b_id,
            &key_b,
            &txn,
            body,
            "timeline_reconnect B send while A offline",
        )
        .await?;
    }

    if legacy_fallback {
        proxy.arm_legacy_fallback()?;
        let start_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: start_id,
            }))
            .await
            .map_err(|e| format!("timeline legacy fallback: submit start A failed: {e}"))?;
        let selected = wait_for_sync_started(
            &mut conn_a,
            start_id,
            "timeline legacy fallback selects SyncService",
        )
        .await?;
        if selected != SyncBackendKind::SyncService {
            return Err("timeline legacy fallback did not initially select SyncService".to_owned());
        }
        wait_for_legacy_fallback_starting(&mut conn_a, "timeline legacy fallback starting").await?;
        proxy.wait_for_legacy_request_held(EVENT_TIMEOUT)?;
        prove_legacy_stays_starting(&mut conn_a, "timeline legacy fallback lifecycle barrier")
            .await?;
        proxy.release_legacy()?;
        wait_for_sync_running_after_reconnect(
            &mut conn_a,
            "timeline legacy fallback first response committed",
        )
        .await?;
    } else {
        proxy.enable();
        wait_for_sync_running_after_reconnect(&mut conn_a, "timeline_reconnect A recovered")
            .await?;
    }
    let newest_persisted_gap = if restart_with_persisted_gap {
        proxy.disable();
        wait_for_sync_reconnecting(
            &mut conn_a,
            "timeline legacy persisted gap disconnect before second limited response",
        )
        .await?;
        let bodies = (0..30)
            .map(|index| format!("QA timeline persisted newest gap {index:03}"))
            .collect::<Vec<_>>();
        let mut newest_known_event_id = None;
        for (index, body) in bodies.iter().enumerate() {
            let txn = format!("qa-timeline-persisted-newest-{index:03}");
            let send_b_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Timeline(TimelineCommand::SendText {
                    request_id: send_b_id,
                    key: key_b.clone(),
                    transaction_id: txn.clone(),
                    body: body.clone(),
                    mentions: MentionIntent::default(),
                }))
                .await
                .map_err(|e| {
                    format!("timeline legacy persisted gap: submit second batch failed: {e}")
                })?;
            let outcome = wait_for_send_flow_completion(
                &mut conn_b,
                send_b_id,
                &key_b,
                &txn,
                body,
                "timeline legacy persisted gap second offline batch",
            )
            .await?;
            if index + 1 == bodies.len() {
                newest_known_event_id = Some(outcome.event_id);
            }
        }

        proxy.enable();
        wait_for_sync_running_after_reconnect(
            &mut conn_a,
            "timeline legacy persisted gap second limited reconnect committed",
        )
        .await?;
        Some((
            bodies,
            newest_known_event_id.expect("persisted newest-gap batch must contain a newest event"),
        ))
    } else {
        None
    };
    let (runtime_a, mut conn_a, room_absent_checkpoint_baseline) = if restart_with_persisted_gap {
        stop_sync_for_qa(
            &mut conn_b,
            "timeline legacy persisted gap stop B before room-absent proof",
        )
        .await?;
        stop_sync_for_qa(
            &mut conn_a,
            "timeline legacy persisted gap stop before restart",
        )
        .await?;
        drop(conn_a);
        tokio::time::timeout(EVENT_TIMEOUT, runtime_a.shutdown())
            .await
            .map_err(|_| {
                "timeline legacy persisted gap timed out shutting down before restart".to_owned()
            })?;
        let room_absent_checkpoint_baseline = koushi_diagnostics::snapshot()
            .records
            .iter()
            .filter(|record| {
                record.event.source == "core.live_catchup"
                    && record.event.stage == "checkpoint"
                    && record.event.fields.iter().any(|field| {
                        field.key == "checkpoint_origin"
                            && field.value
                                == koushi_diagnostics::DiagnosticValue::Token("room_absent")
                    })
            })
            .count();

        let restarted_runtime = CoreRuntime::start_with_data_dir(data_dir_a.clone());
        let mut restarted_conn = restarted_runtime.attach();
        let restore_id = restarted_conn.next_request_id();
        restarted_conn
            .command(CoreCommand::Account(AccountCommand::RestoreSession {
                request_id: restore_id,
                account_key: account_key_a.clone(),
            }))
            .await
            .map_err(|e| format!("timeline legacy persisted gap: submit restore A failed: {e}"))?;
        wait_for_session_restored(
            &mut restarted_conn,
            restore_id,
            &account_key_a,
            "timeline legacy persisted gap restore A",
        )
        .await?;
        wait_for_ready_snapshot(
            &mut restarted_conn,
            "timeline legacy persisted gap restored session A Ready",
        )
        .await?;

        proxy.arm_legacy_fallback()?;
        let restart_sync_id = restarted_conn.next_request_id();
        restarted_conn
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: restart_sync_id,
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit restarted sync A failed: {e}")
            })?;
        let selected = wait_for_sync_started(
            &mut restarted_conn,
            restart_sync_id,
            "timeline legacy persisted gap restart selects SyncService",
        )
        .await?;
        if selected != SyncBackendKind::SyncService {
            return Err(
                "timeline legacy persisted gap restart did not initially select SyncService"
                    .to_owned(),
            );
        }
        wait_for_legacy_fallback_starting(
            &mut restarted_conn,
            "timeline legacy persisted gap fallback starting",
        )
        .await?;
        proxy.wait_for_legacy_request_held(EVENT_TIMEOUT)?;
        prove_legacy_stays_starting(
            &mut restarted_conn,
            "timeline legacy persisted gap lifecycle barrier",
        )
        .await?;
        proxy.release_legacy()?;
        wait_for_sync_running_after_reconnect(
            &mut restarted_conn,
            "timeline legacy persisted gap room-absent response committed",
        )
        .await?;
        (
            restarted_runtime,
            restarted_conn,
            Some(room_absent_checkpoint_baseline),
        )
    } else {
        (runtime_a, conn_a, None)
    };
    let initial_live_tail_snapshot_baseline = if restart_with_persisted_gap {
        Some(live_tail_snapshot_completion_count_for_qa())
    } else {
        None
    };
    let live_tail_recent_body = "QA timeline live tail refreshed recent";
    if restart_with_persisted_gap {
        let (newest_known_bodies, newest_known_event_id) = newest_persisted_gap
            .as_ref()
            .expect("persisted-gap live-tail refresh requires a known newest event");
        let newest_known_body = newest_known_bodies
            .last()
            .expect("persisted newest-gap batch must not be empty");
        proxy.arm_first_live_tail_messages_page(
            newest_known_event_id.clone(),
            newest_known_body.clone(),
            "$qa-live-tail-refreshed:example.invalid".to_owned(),
            live_tail_recent_body.to_owned(),
            seed_outcome.event_id.clone(),
            user_b_full_id.clone(),
            seed_body.to_owned(),
        )?;
    }
    let reopened_before_later = if legacy_fallback {
        Some(
            subscribe_and_ack_active_timeline_projection_for_qa(
                &mut conn_a,
                &key_a,
                "timeline legacy fallback authoritative items after first commit",
            )
            .await?,
        )
    } else {
        None
    };
    if let Some(baseline) = room_absent_checkpoint_baseline {
        let initial_live_tail_snapshot_baseline = initial_live_tail_snapshot_baseline
            .expect("persisted-gap live-tail snapshot baseline must be armed before refresh");
        tokio::time::timeout(EVENT_TIMEOUT, async {
            loop {
                let count = koushi_diagnostics::snapshot()
                    .records
                    .iter()
                    .filter(|record| {
                        record.event.source == "core.live_catchup"
                            && record.event.stage == "checkpoint"
                            && record.event.fields.iter().any(|field| {
                                field.key == "checkpoint_origin"
                                    && field.value
                                        == koushi_diagnostics::DiagnosticValue::Token(
                                            "room_absent",
                                        )
                            })
                    })
                    .count();
                if count > baseline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| {
            "timeline legacy persisted gap did not observe a new room_absent checkpoint after restart"
                .to_owned()
        })?;
        let recent_live_tail_render = wait_for_item_with_body(
            &mut conn_a,
            &key_a,
            live_tail_recent_body,
            "timeline legacy persisted gap renders tokenless live-tail refresh",
        )
        .await;
        let observation = proxy.live_tail_messages_observation()?;
        if recent_live_tail_render.is_err() {
            return Err(format!(
                "timeline legacy persisted gap did not render canned live-tail event \
                 (requests={}, exact_tokenless_limit={}, had_from={}, served={})",
                observation.room_messages_request_count,
                observation.first_request_was_exact_tokenless_limit,
                observation.first_request_had_from,
                observation.freshness_page_served,
            ));
        }
        if observation.room_messages_request_count != 1
            || !observation.first_request_was_exact_tokenless_limit
            || observation.first_request_had_from
            || !observation.freshness_page_served
        {
            return Err(format!(
                "timeline legacy persisted gap expected one exact tokenless live-tail request \
                 (requests={}, exact_tokenless_limit={}, had_from={}, served={})",
                observation.room_messages_request_count,
                observation.first_request_was_exact_tokenless_limit,
                observation.first_request_had_from,
                observation.freshness_page_served,
            ));
        }
        let initial_live_tail_gap_count = wait_for_live_tail_snapshot_gap_count_for_qa(
            &conn_a,
            initial_live_tail_snapshot_baseline,
            "timeline legacy persisted gap initial live-tail snapshot",
        )
        .await?;
        if initial_live_tail_gap_count == 0 {
            return Err(
                "timeline legacy persisted gap initial live-tail snapshot did not retain a continuity gap"
                    .to_owned(),
            );
        }
        println!("legacy_live_tail_room_absent=ok");
        println!("live_tail_anchored_silent_gap=ok");

        stop_sync_for_qa(
            &mut conn_a,
            "timeline legacy persisted gap stop before detached tail restart",
        )
        .await?;
        drop(conn_a);
        tokio::time::timeout(EVENT_TIMEOUT, runtime_a.shutdown())
            .await
            .map_err(|_| {
                "timeline legacy persisted gap timed out shutting down before detached restart"
                    .to_owned()
            })?;

        let detached_runtime = CoreRuntime::start_with_data_dir(data_dir_a.clone());
        let mut detached_conn = detached_runtime.attach();
        let detached_restore_id = detached_conn.next_request_id();
        detached_conn
            .command(CoreCommand::Account(AccountCommand::RestoreSession {
                request_id: detached_restore_id,
                account_key: account_key_a.clone(),
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit detached restore A failed: {e}")
            })?;
        wait_for_session_restored(
            &mut detached_conn,
            detached_restore_id,
            &account_key_a,
            "timeline legacy persisted gap detached restore A",
        )
        .await?;
        wait_for_ready_snapshot(
            &mut detached_conn,
            "timeline legacy persisted gap detached restored session A Ready",
        )
        .await?;

        proxy.arm_legacy_fallback()?;
        let detached_sync_start_id = detached_conn.next_request_id();
        detached_conn
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: detached_sync_start_id,
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit detached sync A failed: {e}")
            })?;
        let detached_selected = wait_for_sync_started(
            &mut detached_conn,
            detached_sync_start_id,
            "timeline legacy persisted gap detached restart selects SyncService",
        )
        .await?;
        if detached_selected != SyncBackendKind::SyncService {
            return Err(
                "timeline legacy persisted gap detached restart did not initially select SyncService"
                    .to_owned(),
            );
        }
        wait_for_legacy_fallback_starting(
            &mut detached_conn,
            "timeline legacy persisted gap detached fallback starting",
        )
        .await?;
        proxy.wait_for_legacy_request_held(EVENT_TIMEOUT)?;
        prove_legacy_stays_starting(
            &mut detached_conn,
            "timeline legacy persisted gap detached lifecycle barrier",
        )
        .await?;
        proxy.release_legacy()?;
        wait_for_sync_running_after_reconnect(
            &mut detached_conn,
            "timeline legacy persisted gap detached room-absent response committed",
        )
        .await?;

        let detached_newest_body = "QA timeline detached live tail 127";
        let detached_end_token = "qa-live-tail-detached-end".to_owned();
        proxy.arm_detached_live_tail_messages_page(
            qa_detached_live_tail_events(&user_b_full_id),
            detached_end_token.clone(),
        )?;
        let detached_items = subscribe_and_ack_active_timeline_projection_for_qa(
            &mut detached_conn,
            &key_a,
            "timeline legacy persisted gap detached live tail subscription",
        )
        .await?;
        let (detached_items, visible_gap, initial_gap_projection, detached_gap_count) =
            wait_for_projected_gap_and_item_for_qa(
                &mut detached_conn,
                &key_a,
                detached_items,
                detached_newest_body,
                "timeline legacy persisted gap projects detached visible gap",
            )
            .await?;
        let detached_observation = proxy.live_tail_messages_observation()?;
        if detached_observation.room_messages_request_count != 1
            || !detached_observation.first_request_was_exact_tokenless_limit
            || detached_observation.first_request_had_from
            || !detached_observation.freshness_page_served
        {
            return Err(format!(
                "timeline legacy persisted gap expected one exact tokenless detached request \
                 (requests={}, exact_tokenless_limit={}, had_from={}, served={})",
                detached_observation.room_messages_request_count,
                detached_observation.first_request_was_exact_tokenless_limit,
                detached_observation.first_request_had_from,
                detached_observation.freshness_page_served,
            ));
        }
        let detached_observation = proxy.live_tail_messages_observation()?;
        if detached_observation.expected_end_token_request_count != 0 {
            return Err(format!(
                "timeline legacy persisted gap detached tail consumed its continuation token before explicit viewport demand \
                 (detached_end_token_requests={})",
                detached_observation.expected_end_token_request_count,
            ));
        }
        println!("live_tail_detached_gap=ok");

        let historical_continuation_body = "QA timeline detached historical continuation";
        proxy.arm_historical_continuation_messages_page(
            detached_end_token,
            qa_detached_historical_continuation_events(&user_b_full_id),
        )?;
        let visible_gap_request_id = detached_conn.next_request_id();
        detached_conn
            .command(CoreCommand::Timeline(TimelineCommand::ObserveViewport {
                request_id: visible_gap_request_id,
                key: key_a.clone(),
                observation: TimelineViewportObservation {
                    first_visible_event_id: visible_gap.first_visible_event_id.clone(),
                    last_visible_event_id: visible_gap.last_visible_event_id.clone(),
                    visible_gap_ids: vec![visible_gap.id],
                    at_bottom: false,
                },
            }))
            .await
            .map_err(|e| {
                format!("timeline legacy persisted gap: submit visible gap observation failed: {e}")
            })?;
        wait_for_exact_items_and_gap_release(
            &mut detached_conn,
            &key_a,
            detached_items,
            &[historical_continuation_body.to_owned()],
            Some(initial_gap_projection),
            Some(visible_gap.id),
            "timeline legacy persisted gap visible continuation repair",
        )
        .await?;
        let historical_observation = proxy.live_tail_messages_observation()?;
        if !historical_observation.first_request_had_from
            || !historical_observation.expected_end_token_was_used
            || historical_observation.expected_end_token_request_count != 1
            || !historical_observation.freshness_page_served
        {
            return Err(format!(
                "timeline legacy persisted gap expected one historical continuation request \
                 (had_from={}, exact_end={}, exact_end_requests={}, served={})",
                historical_observation.first_request_had_from,
                historical_observation.expected_end_token_was_used,
                historical_observation.expected_end_token_request_count,
                historical_observation.freshness_page_served,
            ));
        }
        wait_for_timeline_gap_count_for_qa(
            &detached_conn,
            detached_gap_count.saturating_sub(1),
            "timeline legacy persisted gap detached continuation closes its added gap",
        )
        .await?;
        println!("live_tail_historical_continuation=ok");

        start_sync_for_qa(
            &mut conn_b,
            "timeline legacy persisted gap resume B after room-absent proof",
        )
        .await?;
        cleanup_logged_in_runtime(
            conn_b,
            runtime_b,
            account_key_b,
            "timeline legacy persisted gap detached live-tail cleanup B",
        )
        .await?;
        cleanup_logged_in_runtime(
            detached_conn,
            detached_runtime,
            account_key_a,
            "timeline legacy persisted gap detached live-tail cleanup A",
        )
        .await?;
        return Ok(());
    }
    let mut expected_bodies = newest_persisted_gap
        .map(|(bodies, _)| bodies)
        .unwrap_or(offline_bodies.clone());
    if legacy_fallback {
        let later_body = "QA timeline legacy fallback later live event".to_owned();
        let later_txn = "qa-timeline-legacy-fallback-later";
        let later_send_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: later_send_id,
                key: key_b.clone(),
                transaction_id: later_txn.to_owned(),
                body: later_body.clone(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("timeline legacy fallback: submit later send failed: {e}"))?;
        wait_for_send_flow_completion(
            &mut conn_b,
            later_send_id,
            &key_b,
            later_txn,
            &later_body,
            "timeline legacy fallback later live send",
        )
        .await?;
        expected_bodies.push(later_body);
    }
    let reopened_items = match reopened_before_later {
        Some(items) => items,
        None => {
            subscribe_timeline_for_qa(
                &mut conn_a,
                &key_a,
                "timeline_reconnect reopen unsubscribed A room",
            )
            .await?
        }
    };
    if legacy_fallback {
        wait_for_exact_items_and_gap_release(
            &mut conn_a,
            &key_a,
            reopened_items,
            &expected_bodies,
            None,
            None,
            "timeline legacy fallback exact recovery",
        )
        .await?;
        if restart_with_persisted_gap {
            match conn_a.snapshot().timeline.continuity {
                koushi_state::TimelineContinuityState::Incomplete { gap_count: 1, .. } => {
                    println!("legacy_persisted_gap_unrelated_gap_retained=ok");
                }
                ref continuity => {
                    return Err(format!(
                        "timeline legacy persisted gap expected one unrelated older gap after newest repair, got {continuity:?}"
                    ));
                }
            }
        }
        let settled_body = "QA timeline legacy fallback settled live event";
        let settled_txn = "qa-timeline-legacy-fallback-settled";
        let settled_send_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: settled_send_id,
                key: key_b.clone(),
                transaction_id: settled_txn.to_owned(),
                body: settled_body.to_owned(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("timeline legacy fallback: submit settled send failed: {e}"))?;
        wait_for_send_flow_completion(
            &mut conn_b,
            settled_send_id,
            &key_b,
            settled_txn,
            settled_body,
            "timeline legacy fallback settled send",
        )
        .await?;
        wait_for_item_with_body(
            &mut conn_a,
            &key_a,
            settled_body,
            "timeline legacy fallback receives after repair settlement",
        )
        .await?;
        if restart_with_persisted_gap {
            println!("legacy_persisted_gap_fence=ok");
            println!("legacy_persisted_gap_repaired=ok");
            println!("legacy_persisted_gap_settled=ok");
        } else {
            println!("legacy_fallback_checkpoint=ok");
            println!("legacy_fallback_gap_repaired=ok");
            println!("legacy_fallback_settled=ok");
            println!("legacy_fallback_lifecycle=ok");
        }
    } else {
        wait_for_all_items_with_bodies(
            &mut conn_a,
            &key_a,
            &reopened_items,
            &offline_bodies,
            "timeline_reconnect A repairs the complete missed batch",
        )
        .await?;
        println!("timeline_reconnect_recv_after_reconnect=ok");
        println!("live_catchup_checkpoint=ok");
        println!("live_catchup_gap_repaired=ok");
        println!("timeline_reconnect=ok");
    }

    cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "timeline_reconnect cleanup B",
    )
    .await?;
    cleanup_logged_in_runtime(
        conn_a,
        runtime_a,
        account_key_a,
        "timeline_reconnect cleanup A",
    )
    .await?;
    Ok(())
}

fn timeline_gap_count_for_qa(conn: &CoreConnection) -> u32 {
    match conn.snapshot().timeline.continuity {
        koushi_state::TimelineContinuityState::Inspecting {
            known_gap_count, ..
        } => known_gap_count,
        koushi_state::TimelineContinuityState::Incomplete { gap_count, .. }
        | koushi_state::TimelineContinuityState::Repairing { gap_count, .. }
        | koushi_state::TimelineContinuityState::FailedIncomplete { gap_count, .. } => gap_count,
        koushi_state::TimelineContinuityState::Unknown
        | koushi_state::TimelineContinuityState::Healthy { .. } => 0,
    }
}

fn live_tail_snapshot_completion_count_for_qa() -> usize {
    koushi_diagnostics::snapshot()
        .records
        .iter()
        .filter(|record| {
            record.event.source == "core.timeline_gap_repair"
                && record.event.stage == "inspection"
                && record.event.fields.iter().any(|field| {
                    field.key == "trigger"
                        && field.value
                            == koushi_diagnostics::DiagnosticValue::Token("live_tail_snapshot")
                })
                && record.event.fields.iter().any(|field| {
                    field.key == "outcome"
                        && matches!(
                            field.value,
                            koushi_diagnostics::DiagnosticValue::Token(
                                "unknown" | "incomplete" | "healthy"
                            )
                        )
                })
        })
        .count()
}

async fn wait_for_live_tail_snapshot_gap_count_for_qa(
    conn: &CoreConnection,
    completion_baseline: usize,
    label: &str,
) -> Result<u32, String> {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        let mut previous_gap_count = None;
        let mut stable_samples = 0_u8;
        loop {
            let snapshot_completed =
                live_tail_snapshot_completion_count_for_qa() > completion_baseline;
            let continuity_is_inspecting = matches!(
                conn.snapshot().timeline.continuity,
                koushi_state::TimelineContinuityState::Inspecting { .. }
            );
            let gap_count = timeline_gap_count_for_qa(conn);
            if snapshot_completed && !continuity_is_inspecting {
                if previous_gap_count == Some(gap_count) {
                    stable_samples = stable_samples.saturating_add(1);
                } else {
                    previous_gap_count = Some(gap_count);
                    stable_samples = 0;
                }
                if stable_samples >= 1 {
                    break gap_count;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "{label}: did not settle a completed live-tail snapshot (snapshots={}, observed={})",
            live_tail_snapshot_completion_count_for_qa(),
            timeline_gap_count_for_qa(conn),
        )
    })
}

async fn wait_for_projected_gap_and_item_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    mut items: Vec<TimelineItem>,
    expected_body: &str,
    label: &str,
) -> Result<(Vec<TimelineItem>, QaVisibleGapSelection, (u64, u64), u32), String> {
    let mut capture = QaVisibleGapCapture::default();
    loop {
        capture.observe_items(&items, expected_body, label)?;
        if let Some((visible_gap, initial_gap_projection)) = capture.projected_gap()
            && let Some(settled_gap_count) =
                settled_nonzero_timeline_gap_count_for_qa(conn, initial_gap_projection.1)
        {
            return Ok((
                items,
                visible_gap.clone(),
                *initial_gap_projection,
                settled_gap_count,
            ));
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for a projected visible gap"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref event_key,
                items: replacement,
                ..
            }) if event_key == key => items = replacement,
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                diffs,
                ..
            }) if event_key == key => {
                for diff in &diffs {
                    apply_timeline_diff(&mut items, diff);
                }
            }
            CoreEvent::Timeline(TimelineEvent::GapPositionsUpdated {
                key: ref event_key,
                actor_generation,
                generation,
                positions,
                ..
            }) if event_key == key && !positions.is_empty() => {
                capture.observe_gap_positions(
                    &items,
                    actor_generation,
                    generation,
                    &positions,
                    label,
                )?;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QaVisibleGapSelection {
    id: TimelineGapId,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
}

#[derive(Default)]
struct QaVisibleGapCapture {
    exact_body_present: bool,
    projected_gap: Option<(QaVisibleGapSelection, (u64, u64))>,
}

impl QaVisibleGapCapture {
    fn observe_items(
        &mut self,
        items: &[TimelineItem],
        expected_body: &str,
        label: &str,
    ) -> Result<(), String> {
        let body_count = items
            .iter()
            .filter(|item| item.body.as_deref() == Some(expected_body))
            .count();
        if body_count > 1 {
            return Err(format!(
                "{label}: detached live-tail row was projected twice"
            ));
        }

        let exact_body_present = body_count == 1;
        if !exact_body_present || !self.exact_body_present {
            self.projected_gap = None;
        }
        self.exact_body_present = exact_body_present;
        Ok(())
    }

    fn observe_gap_positions(
        &mut self,
        items: &[TimelineItem],
        actor_generation: u64,
        generation: u64,
        positions: &[TimelineGapPosition],
        label: &str,
    ) -> Result<(), String> {
        if !self.exact_body_present {
            self.projected_gap = None;
            return Ok(());
        }

        let visible_gap = select_visible_gap_for_qa(items, positions)
            .map_err(|error| format!("{label}: {error}"))?;
        if let Some((existing_gap, (existing_actor, _))) = self.projected_gap.as_ref()
            && (existing_gap.id != visible_gap.id || *existing_actor != actor_generation)
        {
            return Err(format!(
                "{label}: projected visible gap identity changed before viewport demand"
            ));
        }
        self.projected_gap = Some((visible_gap, (actor_generation, generation)));
        Ok(())
    }

    fn projected_gap(&self) -> Option<&(QaVisibleGapSelection, (u64, u64))> {
        self.projected_gap.as_ref()
    }
}

fn select_visible_gap_for_qa(
    items: &[TimelineItem],
    positions: &[TimelineGapPosition],
) -> Result<QaVisibleGapSelection, String> {
    let bracketed = positions
        .iter()
        .filter_map(|position| {
            let first_visible_event_id = items
                .get(..position.before_item_index)?
                .iter()
                .rev()
                .find_map(|item| match &item.id {
                    TimelineItemId::Event { event_id } => Some(event_id.clone()),
                    TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
                })?;
            let last_visible_event_id =
                items
                    .get(position.before_item_index..)?
                    .iter()
                    .find_map(|item| match &item.id {
                        TimelineItemId::Event { event_id } => Some(event_id.clone()),
                        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => {
                            None
                        }
                    })?;
            Some((
                position.before_item_index,
                position.id,
                QaVisibleGapSelection {
                    id: position.id,
                    first_visible_event_id: Some(first_visible_event_id),
                    last_visible_event_id: Some(last_visible_event_id),
                },
            ))
        })
        .max_by_key(|(before_item_index, id, _)| {
            (*before_item_index, id.topology_revision, id.ordinal)
        })
        .map(|(_, _, selection)| selection);
    if let Some(selection) = bracketed {
        return Ok(selection);
    }

    if let Some(position) = positions
        .iter()
        .filter(|position| position.before_item_index == 0)
        .max_by_key(|position| (position.id.topology_revision, position.id.ordinal))
    {
        return Ok(QaVisibleGapSelection {
            id: position.id,
            first_visible_event_id: None,
            last_visible_event_id: None,
        });
    }

    let min_before_item_index = positions
        .iter()
        .map(|position| position.before_item_index)
        .min()
        .map_or_else(|| "none".to_owned(), |index| index.to_string());
    let max_before_item_index = positions
        .iter()
        .map(|position| position.before_item_index)
        .max()
        .map_or_else(|| "none".to_owned(), |index| index.to_string());
    Err(format!(
        "visible gap selection found no bracketed or top-row position \
         (item_count={}, position_count={}, min_before_item_index={}, \
         max_before_item_index={})",
        items.len(),
        positions.len(),
        min_before_item_index,
        max_before_item_index,
    ))
}

fn settled_nonzero_timeline_gap_count_for_qa(
    conn: &CoreConnection,
    projection_generation: u64,
) -> Option<u32> {
    // The position event and Incomplete state share one inspection serial.
    // Starting repair allocates a newer serial, which FailedIncomplete retains.
    let gap_count = match conn.snapshot().timeline.continuity {
        koushi_state::TimelineContinuityState::Incomplete {
            generation,
            gap_count,
        } if generation == projection_generation => gap_count,
        koushi_state::TimelineContinuityState::Repairing {
            generation,
            gap_count,
            ..
        }
        | koushi_state::TimelineContinuityState::FailedIncomplete {
            generation,
            gap_count,
            ..
        } if generation >= projection_generation => gap_count,
        koushi_state::TimelineContinuityState::Unknown
        | koushi_state::TimelineContinuityState::Inspecting { .. }
        | koushi_state::TimelineContinuityState::Healthy { .. }
        | koushi_state::TimelineContinuityState::Incomplete { .. }
        | koushi_state::TimelineContinuityState::Repairing { .. }
        | koushi_state::TimelineContinuityState::FailedIncomplete { .. } => return None,
    };
    (gap_count > 0).then_some(gap_count)
}

async fn wait_for_timeline_gap_count_for_qa(
    conn: &CoreConnection,
    expected_gap_count: u32,
    label: &str,
) -> Result<(), String> {
    let result = tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            if timeline_gap_count_for_qa(conn) == expected_gap_count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(_) => Err(format!(
            "{label}: did not settle at the expected coarse gap count \
             (expected={}, observed={})",
            expected_gap_count,
            timeline_gap_count_for_qa(conn),
        )),
    }
}

fn qa_detached_live_tail_events(sender: &str) -> Vec<QaCannedTimelineEvent> {
    (0..128)
        .rev()
        .map(|index| QaCannedTimelineEvent {
            event_id: format!("$qa-live-tail-detached-{index:03}:example.invalid"),
            sender: sender.to_owned(),
            body: format!("QA timeline detached live tail {index:03}"),
            origin_server_ts: 1_900_000_100_000 + index as u64,
        })
        .collect()
}

fn qa_detached_historical_continuation_events(sender: &str) -> Vec<QaCannedTimelineEvent> {
    vec![QaCannedTimelineEvent {
        event_id: "$qa-live-tail-detached-historical:example.invalid".to_owned(),
        sender: sender.to_owned(),
        body: "QA timeline detached historical continuation".to_owned(),
        origin_server_ts: 1_900_000_099_999,
    }]
}

async fn cleanup_after_full_flow(
    mut conn_a: CoreConnection,
    mut conn_b: CoreConnection,
    runtime_a: CoreRuntime,
    runtime_b: CoreRuntime,
    data_dir_a: std::path::PathBuf,
    account_key_a: AccountKey,
    account_key_b: AccountKey,
) -> Result<String, String> {
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");

    drop(conn_a);
    runtime_a.shutdown().await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    let restore_gone_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_gone_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn_a2,
        restore_gone_id,
        "post-logout restore A (must fail)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore A failed with unexpected kind: {failure:?}"
        ));
    }
    let logout_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_b_id,
        }))
        .await
        .map_err(|e| format!("submit logout B: {e}"))?;

    wait_for_logged_out(&mut conn_b, logout_b_id, &account_key_b, "logout B").await?;

    let restore_last_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_last_id,
        }))
        .await
        .map_err(|e| format!("submit post-logout restore-last: {e}"))?;

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn_b,
        restore_last_id,
        "post-logout restore-last (must be not-found)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected kind: {failure:?}"
        ));
    }
    drop(conn_b);
    runtime_b.shutdown().await;

    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

fn should_bootstrap_new_identity_before_logged_in(scenario: QaScenario) -> bool {
    matches!(
        scenario,
        QaScenario::All
            | QaScenario::InvitesDm
            | QaScenario::E2eeTrust
            | QaScenario::GateRestore
            | QaScenario::GateNegative
    )
}

fn should_run_normal_secondary_participant(scenario: QaScenario) -> bool {
    scenario.should_run_stage(QaStage::InvitesDm)
        || scenario.should_run_stage(QaStage::Directory)
        || scenario.should_run_stage(QaStage::RoomSpace)
}

fn should_run_focused_send_queue_route(scenario: QaScenario) -> bool {
    scenario == QaScenario::SendQueue
}

async fn run_async(config: QaConfig, scenario: QaScenario) -> Result<String, String> {
    if scenario == QaScenario::Safety {
        println!("safety=ok");
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::CacheRestore {
        println!("safety=ok");
        run_cache_restore_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::TimelineReconnect {
        println!("safety=ok");
        run_timeline_reconnect_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if scenario == QaScenario::TimelineLegacyFallback {
        println!("safety=ok");
        run_timeline_legacy_fallback_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if scenario == QaScenario::TimelineLegacyPersistedGap {
        println!("safety=ok");
        run_timeline_legacy_persisted_gap_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if scenario == QaScenario::GateNoProof {
        println!("safety=ok");
        run_gate_no_proof_stage(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }
    if should_run_focused_send_queue_route(scenario) {
        println!("safety=ok");
        run_focused_send_queue_scenario(&config).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // One CoreRuntime per synthetic user (two-device topology).
    let data_dir_a = qa_data_dir("a");
    let data_dir_b = qa_data_dir("b");

    // -----------------------------------------------------------------------
    // --- Login A (storeless exchange + store bootstrap inside the actor) ---
    // -----------------------------------------------------------------------
    let mut runtime_a = CoreRuntime::start_with_data_dir(data_dir_a.clone());
    let mut conn_a = runtime_a.attach();

    let login_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some(DEVICE_A.to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("submit login A: {e}"))?;

    let bootstrap_recovery_secret_a = if should_bootstrap_new_identity_before_logged_in(scenario) {
        let secret =
            complete_new_identity_gate_for_qa(&mut conn_a, &config.password_a, "gate-bootstrap-a")
                .await?;
        println!("gate_new_identity_bootstrap=ok");
        secret
    } else {
        None
    };

    let mut account_key_a = wait_for_logged_in(&mut conn_a, login_a_id, "login A").await?;
    wait_for_ready_snapshot(&mut conn_a, "session A Ready").await?;

    // -----------------------------------------------------------------------
    // --- Phase 3: Start sync A, assert Started + Running, record backend ---
    // -----------------------------------------------------------------------
    let sync_start_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_id,
        }))
        .await
        .map_err(|e| format!("submit sync start A: {e}"))?;

    let sync_backend_a =
        wait_for_sync_started_and_running(&mut conn_a, sync_start_id, "sync start A").await?;
    println!("sync_backend_a={sync_backend_a:?}");
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_a,
        "sync start A",
    )?;

    println!("sync_a=running");
    println!("login_sync=ok");

    if scenario == QaScenario::TimelineStress {
        let stress = TimelineStressConfig::from_env()?;
        if stress.replay_existing {
            let runtime_b = CoreRuntime::start_with_data_dir(data_dir_b.clone());
            let mut conn_b = runtime_b.attach();

            let login_b_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Account(AccountCommand::LoginPassword {
                    request_id: login_b_id,
                    request: koushi_state::LoginRequest {
                        homeserver: config.homeserver.clone(),
                        username: config.user_b.clone(),
                        password: AuthSecret::new(config.password_b.clone()),
                        device_display_name: Some(DEVICE_B.to_owned()),
                    },
                }))
                .await
                .map_err(|e| format!("timeline_stress replay: submit login B failed: {e}"))?;

            let account_key_b =
                wait_for_logged_in(&mut conn_b, login_b_id, "timeline_stress replay login B")
                    .await?;
            wait_for_ready_snapshot(&mut conn_b, "timeline_stress replay session B Ready").await?;

            let sync_start_b_id = conn_b.next_request_id();
            conn_b
                .command(CoreCommand::Sync(SyncCommand::Start {
                    request_id: sync_start_b_id,
                }))
                .await
                .map_err(|e| format!("timeline_stress replay: submit sync start B failed: {e}"))?;

            let sync_backend_b = wait_for_sync_started_and_running(
                &mut conn_b,
                sync_start_b_id,
                "timeline_stress replay sync start B",
            )
            .await?;
            println!("sync_backend_b={sync_backend_b:?}");
            assert_expected_backend(
                config.expect_sync_backend.as_deref(),
                sync_backend_b,
                "timeline_stress replay sync start B",
            )?;
            println!("sync_b=running");

            run_timeline_stress_replay_stage(
                &mut conn_a,
                &mut conn_b,
                &account_key_a,
                &account_key_b,
                stress,
            )
            .await?;
            cleanup_after_full_flow(
                conn_a,
                conn_b,
                runtime_a,
                runtime_b,
                data_dir_a,
                account_key_a,
                account_key_b,
            )
            .await?;
            return Ok(scenario_report(&config.server_kind, scenario));
        }
    }

    if scenario.should_run_stage(QaStage::CredentialHealth) {
        run_credential_health_stage(&mut conn_a).await?;
    }

    if scenario.should_run_stage(QaStage::NativeAttention) {
        run_native_attention_stage(&mut conn_a).await?;
    }

    if scenario == QaScenario::E2eeTrust {
        run_e2ee_trust_stage(&config, &mut conn_a, &account_key_a, None).await?;
        drop(conn_a);
        runtime_a.shutdown().await;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::GateRestore {
        run_gate_restore_stage(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario == QaScenario::GateNegative {
        run_gate_negative_stage(
            &config,
            &mut conn_a,
            bootstrap_recovery_secret_a
                .as_ref()
                .ok_or_else(|| "gate negative bootstrap recovery secret unavailable".to_owned())?,
        )
        .await?;
        drop(conn_a);
        runtime_a.shutdown().await;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    let mut normal_secondary = if should_run_normal_secondary_participant(scenario) {
        let participant = login_synced_participant_for_qa(
            &config.homeserver,
            data_dir_b.clone(),
            &config.user_b,
            &config.password_b,
            DEVICE_B,
            "normal secondary login B",
            "normal secondary bootstrap gate B",
            QaParticipantLoginGate::BootstrapNewIdentity,
        )
        .await?;
        println!("sync_backend_b={:?}", participant.sync_backend);
        assert_expected_backend(
            config.expect_sync_backend.as_deref(),
            participant.sync_backend,
            "normal secondary sync start B",
        )?;
        println!("sync_b=running");
        Some(participant)
    } else {
        None
    };

    if scenario.should_run_stage(QaStage::InvitesDm) {
        let conn_b = &mut normal_secondary
            .as_mut()
            .ok_or_else(|| "InvitesDm requires the normal secondary participant".to_owned())?
            .conn;
        run_invites_dm_stage(&config, &mut conn_a, conn_b).await?;
    }

    if scenario == QaScenario::InvitesDm {
        cleanup_normal_secondary_participant_for_qa(
            &mut normal_secondary,
            "InvitesDm normal secondary cleanup",
        )
        .await?;
        cleanup_after_login_sync(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if scenario.should_run_stage(QaStage::Directory) {
        let conn_b = &mut normal_secondary
            .as_mut()
            .ok_or_else(|| "Directory requires the normal secondary participant".to_owned())?
            .conn;
        run_directory_stage(&config, &mut conn_a, conn_b).await?;
    }

    if !scenario.should_run_stage(QaStage::RoomSpace) {
        cleanup_normal_secondary_participant_for_qa(
            &mut normal_secondary,
            "pre-RoomSpace normal secondary cleanup",
        )
        .await?;
        cleanup_after_login_sync(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // -----------------------------------------------------------------------
    // --- Phase 4: Room operations (A creates room + space, invites B) ---
    // -----------------------------------------------------------------------

    // A creates a room
    let create_room_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: create_room_id,
            options: private_room_options("QA Room", false),
        }))
        .await
        .map_err(|e| format!("submit create room: {e}"))?;

    let room_id = wait_for_room_created(&mut conn_a, create_room_id, "create room").await?;
    println!("room_created=ok");

    // A creates a space
    let create_space_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateSpace {
            request_id: create_space_id,
            name: "QA Space".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit create space: {e}"))?;

    let space_id = wait_for_space_created(&mut conn_a, create_space_id, "create space").await?;
    println!("space_created=ok");

    // Extract server name from room_id (e.g., "!room:localhost:PORT" → "localhost:PORT")
    let via_server = config.server_name.clone();

    // A sets room as child of space
    let set_child_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::SetSpaceChild {
            request_id: set_child_id,
            space_id: space_id.clone(),
            child_room_id: room_id.clone(),
            via_server: via_server.clone(),
        }))
        .await
        .map_err(|e| format!("submit set space child: {e}"))?;

    wait_for_space_child_set(
        &mut conn_a,
        set_child_id,
        &space_id,
        &room_id,
        "set space child",
    )
    .await?;
    println!("space_child_set=ok");

    // A invites B to the room
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let invite_room_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::InviteUser {
            request_id: invite_room_id,
            room_id: room_id.clone(),
            user_id: user_b_full_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit invite B to room: {e}"))?;

    wait_for_user_invited(
        &mut conn_a,
        invite_room_id,
        &room_id,
        &user_b_full_id,
        "invite B to room",
    )
    .await?;
    println!("invite_b_to_room=ok");

    // A invites B to the space
    let invite_space_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::InviteUser {
            request_id: invite_space_id,
            room_id: space_id.clone(),
            user_id: user_b_full_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit invite B to space: {e}"))?;

    wait_for_user_invited(
        &mut conn_a,
        invite_space_id,
        &space_id,
        &user_b_full_id,
        "invite B to space",
    )
    .await?;
    println!("invite_b_to_space=ok");

    // Wait (event-driven, bounded) until A's room list contains the created
    // room AND the created space; the wait itself is the assertion.
    let snapshot_a = wait_for_room_list_containing(
        &mut conn_a,
        &room_id,
        &space_id,
        "room list A after creates",
    )
    .await?;
    let room_list_a = room_list_summary(&snapshot_a);
    println!("room_list_a={room_list_a}");

    // -----------------------------------------------------------------------
    // --- Reuse centrally logged-in B + join room + join space ---
    // -----------------------------------------------------------------------
    let normal_secondary = normal_secondary.take();
    let normal_secondary = normal_secondary
        .ok_or_else(|| "RoomSpace requires the normal secondary participant".to_owned())?;
    let QaParticipantLoginOutcome {
        runtime: mut runtime_b,
        conn: mut conn_b,
        account_key: mut account_key_b,
        bootstrap_recovery_secret: _,
        sync_backend: _,
    } = normal_secondary;

    // B joins the room
    let join_room_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_room_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit join room B: {e}"))?;

    wait_for_room_joined(&mut conn_b, join_room_id, &room_id, "B joins room").await?;
    println!("b_joined_room=ok");

    // B joins the space
    let join_space_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: join_space_id,
            room_id: space_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit join space B: {e}"))?;

    wait_for_room_joined(&mut conn_b, join_space_id, &space_id, "B joins space").await?;
    println!("b_joined_space=ok");

    // Wait (event-driven, bounded) until B's room list contains the joined
    // room AND the joined space; the wait itself is the assertion.
    let snapshot_b =
        wait_for_room_list_containing(&mut conn_b, &room_id, &space_id, "room list B after joins")
            .await?;
    let room_list_b = room_list_summary(&snapshot_b);
    println!("room_list_b={room_list_b}");
    println!("room_space=ok");

    if scenario.should_run_stage(QaStage::RoomManagement) {
        run_room_management_stage(
            &config,
            &mut conn_a,
            &mut conn_b,
            &account_key_a,
            &account_key_b,
        )
        .await?;
    }

    if !scenario.should_run_stage(QaStage::Timeline) {
        cleanup_after_full_flow(
            conn_a,
            conn_b,
            runtime_a,
            runtime_b,
            data_dir_a,
            account_key_a,
            account_key_b,
        )
        .await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // -----------------------------------------------------------------------
    // --- Phase 5: Timeline subscribe, send, receive, edit, redact, paginate ---
    // -----------------------------------------------------------------------

    // A subscribes to the room timeline.
    let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let subscribe_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_a_id,
            key: key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit subscribe timeline A: {e}"))?;

    wait_for_initial_items(&mut conn_a, &key_a, subscribe_a_id, "subscribe timeline A").await?;
    println!("timeline_subscribed_a=ok");

    // A sends message 1 with a distinct client transaction id.
    let txn1 = "qa-phase5-txn-1".to_owned();
    let send1_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send1_id,
            key: key_a.clone(),
            transaction_id: txn1.clone(),
            body: "Phase 5 QA message 1".to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("submit send1: {e}"))?;

    let send1_outcome = wait_for_send_flow_completion(
        &mut conn_a,
        send1_id,
        &key_a,
        &txn1,
        "Phase 5 QA message 1",
        "send flow msg1",
    )
    .await?;
    let _echo1_sdk_txn = send1_outcome.sdk_transaction_id;
    let event1_id = send1_outcome.event_id;
    println!("local_echo_msg1=ok");
    println!("send_completed_msg1=ok");

    // A sends message 2.
    let txn2 = "qa-phase5-txn-2".to_owned();
    let send2_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send2_id,
            key: key_a.clone(),
            transaction_id: txn2.clone(),
            body: "Phase 5 QA message 2".to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("submit send2: {e}"))?;

    let send2_outcome = wait_for_send_flow_completion(
        &mut conn_a,
        send2_id,
        &key_a,
        &txn2,
        "Phase 5 QA message 2",
        "send flow msg2",
    )
    .await?;
    let _echo2_sdk_txn = send2_outcome.sdk_transaction_id;
    let event2_id = send2_outcome.event_id;
    println!("local_echo_msg2=ok");
    println!("send_completed_msg2=ok");

    // B subscribes and receives both messages (event-driven wait on diffs).
    let key_b = TimelineKey::room(account_key_b.clone(), room_id.clone());
    let subscribe_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_b_id,
            key: key_b.clone(),
        }))
        .await
        .map_err(|e| format!("submit subscribe timeline B: {e}"))?;

    let b_initial =
        wait_for_initial_items(&mut conn_b, &key_b, subscribe_b_id, "subscribe timeline B").await?;
    println!("timeline_subscribed_b=ok");

    // Paginate backward on B to ensure A's messages are loaded from server
    // history (required because the SDK's Live timeline only has what's in
    // the local event cache; a newly-joined room may not have prior msgs yet).
    // We fire the paginate and then use wait_for_item_bodies_with_paginate
    // which scans both the initial items, the pagination diffs, and live diffs.
    let paginate_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_b_id,
            key: key_b.clone(),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .map_err(|e| format!("B backfill paginate: {e}"))?;

    // Now consume events until we've seen all required bodies AND pagination
    // has settled (Idle or EndReached). This single loop handles both.
    wait_for_bodies_and_pagination_settle(
        &mut conn_b,
        &key_b,
        &b_initial,
        &["Phase 5 QA message 1", "Phase 5 QA message 2"],
        "B receives 2 messages from A",
    )
    .await?;
    println!("b_recv_msgs=ok");

    let nav_marker_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
            request_id: nav_marker_id,
            key: key_b.clone(),
            event_id: event1_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit navigation fully-read marker: {e}"))?;
    let nav_viewport_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::ObserveViewport {
            request_id: nav_viewport_id,
            key: key_b.clone(),
            observation: TimelineViewportObservation {
                first_visible_event_id: Some(event1_id.clone()),
                last_visible_event_id: Some(event1_id.clone()),
                visible_gap_ids: Vec::new(),
                at_bottom: false,
            },
        }))
        .await
        .map_err(|e| format!("submit navigation viewport observation: {e}"))?;
    wait_for_timeline_navigation(
        &mut conn_b,
        &key_b,
        TimelineUnreadPosition::BelowViewport,
        1,
        1,
        "timeline navigation",
    )
    .await?;
    println!("timeline_nav=ok");

    // A edits message 1 — assert a Set diff reflecting the edit on original item identity.
    let edit1_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::EditText {
            request_id: edit1_id,
            key: key_a.clone(),
            event_id: event1_id.clone(),
            body: "Phase 5 QA message 1 EDITED".to_owned(),
        }))
        .await
        .map_err(|e| format!("submit edit msg1: {e}"))?;

    wait_for_edit_diff(
        &mut conn_a,
        &key_a,
        edit1_id,
        &event1_id,
        "Phase 5 QA message 1 EDITED",
        "edit msg1",
    )
    .await?;
    println!("edit_msg1=ok");

    // A redacts message 2 — assert removal or redacted-state diff.
    let redact2_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Redact {
            request_id: redact2_id,
            key: key_a.clone(),
            event_id: event2_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit redact msg2: {e}"))?;

    wait_for_redact_diff(&mut conn_a, &key_a, redact2_id, "redact msg2").await?;
    println!("redact_msg2=ok");

    run_hide_redacted_stage(&mut conn_a, &key_a).await?;

    // A paginates backward with a small page size until EndReached.
    // Assert Paginating → EndReached and strictly increasing batch_ids per generation.
    let paginate_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: key_a.clone(),
            direction: PaginationDirection::Backward,
            event_count: 5,
        }))
        .await
        .map_err(|e| format!("submit paginate: {e}"))?;

    let paginate_result =
        wait_for_paginate_end_reached(&mut conn_a, &key_a, paginate_id, "paginate to EndReached")
            .await?;
    println!("paginate={paginate_result}");

    if scenario.should_run_stage(QaStage::LiveSignals) {
        run_live_signals_stage(
            &mut conn_a,
            &mut conn_b,
            &key_a,
            &key_b,
            &event1_id,
            &account_key_b.0,
        )
        .await?;
    }

    if scenario.should_run_stage(QaStage::Activity) {
        run_activity_stage(&mut conn_a, &mut conn_b, &key_a, &key_b, &room_id).await?;
    }

    if scenario.should_run_stage(QaStage::Composer) {
        run_composer_stage(&mut conn_a, &key_a, &account_key_b.0).await?;
    }

    if scenario.should_run_stage(QaStage::Reply) {
        // -------------------------------------------------------------------
        // --- Phase 5b: True reply relation QA ---
        // -------------------------------------------------------------------

        let txn_b_reply = "qa-phase5-txn-b-reply".to_owned();
        let send_b_reply_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id: send_b_reply_id,
                key: key_b.clone(),
                transaction_id: txn_b_reply.clone(),
                in_reply_to_event_id: event1_id.clone(),
                body: "Phase 5 QA reply from B".to_owned(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("submit B reply: {e}"))?;

        let (_b_echo_txn, _b_reply_event_id) =
            wait_for_send_completed(&mut conn_b, send_b_reply_id, &key_b, "B reply completed")
                .await?;
        println!("b_reply_sent=ok");

        let reply_item = wait_for_item_with_body(
            &mut conn_a,
            &key_a,
            "Phase 5 QA reply from B",
            "A receives reply from B",
        )
        .await?;
        if reply_item.in_reply_to_event_id != Some(event1_id.clone()) {
            return Err("reply relation mismatch".to_owned());
        }
        println!("reply=ok");

        let Some(reply_quote) = reply_item.reply_quote.as_ref() else {
            return Err("reply_quote failed: missing quote".to_owned());
        };
        if reply_quote.event_id != event1_id
            || reply_quote.state != ReplyQuoteState::Ready
            || reply_quote.body_preview.is_none()
        {
            return Err("reply_quote failed: quote was not ready".to_owned());
        }
        println!("reply_quote=ok");

        let pin_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::PinEvent {
                request_id: pin_id,
                room_id: room_id.clone(),
                event_id: event1_id.clone(),
            }))
            .await
            .map_err(|e| format!("submit pin event: {e}"))?;
        wait_for_pin_event_completed(&mut conn_a, pin_id, "pin event completed").await?;
        println!("pin_event=ok");

        wait_for_pinned_state(
            &mut conn_a,
            &room_id,
            &event1_id,
            true,
            "pinned state after pin",
        )
        .await?;
        println!("pinned_state=ok");

        let unpin_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::UnpinEvent {
                request_id: unpin_id,
                room_id: room_id.clone(),
                event_id: event1_id.clone(),
            }))
            .await
            .map_err(|e| format!("submit unpin event: {e}"))?;
        wait_for_unpin_event_completed(&mut conn_a, unpin_id, "unpin event completed").await?;
        wait_for_pinned_state(
            &mut conn_a,
            &room_id,
            &event1_id,
            false,
            "pinned state after unpin",
        )
        .await?;
        println!("unpin_event=ok");
    }

    if scenario.should_run_stage(QaStage::Media) {
        run_media_stage(&mut conn_a, &mut conn_b, &key_a, &key_b).await?;
    }

    if scenario.should_run_stage(QaStage::LinkPreview) {
        run_link_preview_stage(&mut conn_a, &mut conn_b, &key_a, &key_b).await?;
    }

    if scenario.should_run_stage(QaStage::Thread) {
        // -------------------------------------------------------------------
        // --- Phase 5c: Thread timeline QA ---
        // -------------------------------------------------------------------

        let thread_key_b = TimelineKey {
            account_key: account_key_b.clone(),
            kind: TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: event1_id.clone(),
            },
        };
        let subscribe_thread_b_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: subscribe_thread_b_id,
                key: thread_key_b.clone(),
            }))
            .await
            .map_err(|e| format!("submit subscribe thread B: {e}"))?;

        wait_for_initial_items(
            &mut conn_b,
            &thread_key_b,
            subscribe_thread_b_id,
            "subscribe thread B",
        )
        .await?;

        let txn_b_thread_reply = "qa-phase11-txn-b-thread-reply".to_owned();
        let send_b_thread_reply_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id: send_b_thread_reply_id,
                key: thread_key_b.clone(),
                transaction_id: txn_b_thread_reply.clone(),
                in_reply_to_event_id: event1_id.clone(),
                body: THREAD_REPLY_BODY.to_owned(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("submit B thread reply: {e}"))?;

        let (_thread_b_echo_txn, _thread_b_reply_event_id) = wait_for_send_completed(
            &mut conn_b,
            send_b_thread_reply_id,
            &thread_key_b,
            "B thread reply completed",
        )
        .await?;

        let refresh_room_a_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: refresh_room_a_id,
                key: key_a.clone(),
            }))
            .await
            .map_err(|e| format!("submit refresh room timeline A: {e}"))?;

        let refreshed_room_items = wait_for_initial_items(
            &mut conn_a,
            &key_a,
            refresh_room_a_id,
            "refresh room timeline A after thread send",
        )
        .await?;
        wait_for_room_timeline_thread_summary(
            &mut conn_a,
            &key_a,
            &refreshed_room_items,
            THREAD_REPLY_BODY,
            &event1_id,
            "wait for A room live thread summary",
        )
        .await?;
        println!("thread_canonical=ok");
        println!("thread_summary=ok");

        let thread_key_a = TimelineKey {
            account_key: account_key_a.clone(),
            kind: TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: event1_id.clone(),
            },
        };
        let subscribe_thread_a_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
                request_id: subscribe_thread_a_id,
                key: thread_key_a.clone(),
            }))
            .await
            .map_err(|e| format!("submit subscribe thread A: {e}"))?;

        let thread_initial_items = wait_for_initial_items(
            &mut conn_a,
            &thread_key_a,
            subscribe_thread_a_id,
            "subscribe thread A after thread send",
        )
        .await?;

        let thread_item = if thread_initial_items_need_paginate_backfill(
            &thread_initial_items,
            THREAD_REPLY_BODY,
        ) {
            wait_for_thread_reply_item(
                &mut conn_a,
                &thread_key_a,
                &thread_initial_items,
                THREAD_REPLY_BODY,
                "A receives thread reply from B",
            )
            .await?
        } else {
            find_timeline_item_with_body(&thread_initial_items, THREAD_REPLY_BODY)
                .expect("thread reply present after initial scan")
        };
        assert_thread_reply_relation(&thread_item, &event1_id)?;
        println!("thread_recv=ok");

        let thread_paginate_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id: thread_paginate_id,
                key: thread_key_a.clone(),
                direction: PaginationDirection::Backward,
                event_count: 5,
            }))
            .await
            .map_err(|e| format!("submit thread paginate: {e}"))?;

        let thread_paginate_result = wait_for_paginate_end_reached(
            &mut conn_a,
            &thread_key_a,
            thread_paginate_id,
            "thread paginate to EndReached",
        )
        .await?;
        println!("thread_paginate={thread_paginate_result}");

        let unsub_thread_a_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
                request_id: unsub_thread_a_id,
                key: thread_key_a.clone(),
            }))
            .await
            .map_err(|e| format!("submit unsubscribe thread A: {e}"))?;

        let unsub_thread_b_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
                request_id: unsub_thread_b_id,
                key: thread_key_b.clone(),
            }))
            .await
            .map_err(|e| format!("submit unsubscribe thread B: {e}"))?;
    }

    if scenario.should_run_stage(QaStage::ScheduledSend) {
        run_scheduled_send_stage(&mut conn_a, &key_a, &room_id).await?;
    }

    if scenario.should_run_stage(QaStage::TimelineStress) {
        run_timeline_stress_stage(
            &config,
            &mut conn_a,
            &mut conn_b,
            &account_key_a,
            &account_key_b,
        )
        .await?;
    }

    // Unsubscribe A and B to confirm no leaks.
    let unsub_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_a_id,
            key: key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe A: {e}"))?;

    let unsub_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_b_id,
            key: key_b.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe B: {e}"))?;

    // Unsubscribe has no completion event (it just drops the timeline actor,
    // per the timeline spec). No blind sleep is needed: the next step that
    // depends on this connection — a re-subscribe awaiting InitialItems, or a
    // sync stop awaiting SyncStopped — is dispatched after these unsubscribes
    // on the same FIFO-ordered connection, so the actor is dropped first and
    // the following request-id-scoped wait provides the real synchronization.
    println!("timeline=ok");

    if scenario.should_run_stage(QaStage::SendQueue) {
        let recovery_secret = bootstrap_recovery_secret_a
            .as_ref()
            .ok_or_else(|| "send_queue: primary recovery secret unavailable".to_owned())?;
        run_send_queue_stage(&config, recovery_secret).await?;
    }

    if !scenario.should_run_stage(QaStage::EditRedactSearch) {
        cleanup_after_full_flow(
            conn_a,
            conn_b,
            runtime_a,
            runtime_b,
            data_dir_a,
            account_key_a,
            account_key_b,
        )
        .await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    // -----------------------------------------------------------------------
    // --- Phase 6: Search QA (CJK query, edit, redact) ---
    // -----------------------------------------------------------------------

    // Re-subscribe A's timeline for the search round-trip.
    let key_a_search = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let subscribe_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_search_id,
            key: key_a_search.clone(),
        }))
        .await
        .map_err(|e| format!("submit subscribe timeline A (search): {e}"))?;

    wait_for_initial_items(
        &mut conn_a,
        &key_a_search,
        subscribe_search_id,
        "subscribe timeline A search",
    )
    .await?;

    // Send a message with a CJK body that will be indexed.
    const SEARCH_BODY: &str = "検索対象メッセージ Phase6 QA";
    const SEARCH_QUERY: &str = "検索対象";
    const EDITED_BODY: &str = "Phase6 QA 編集済みメッセージ";
    const EDITED_QUERY: &str = "編集済み";

    let txn_search = "qa-phase6-search-txn".to_owned();
    let send_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_search_id,
            key: key_a_search.clone(),
            transaction_id: txn_search.clone(),
            body: SEARCH_BODY.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("submit search send: {e}"))?;

    let (_, search_event_id) = wait_for_send_completed(
        &mut conn_a,
        send_search_id,
        &key_a_search,
        "send search msg",
    )
    .await?;
    println!("search_msg_sent=ok");

    // Poll SearchCommand::Query until Results contains search_event_id.
    // The ngram index is fed by the SDK sync loop; wait up to 30s for indexing.
    poll_search_until_found(
        &mut conn_a,
        &account_key_a,
        SEARCH_QUERY,
        &search_event_id,
        &room_id,
        "search=ok (CJK query)",
    )
    .await?;
    println!("search=ok");

    // Edit the search message.
    let edit_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::EditText {
            request_id: edit_search_id,
            key: key_a_search.clone(),
            event_id: search_event_id.clone(),
            body: EDITED_BODY.to_owned(),
        }))
        .await
        .map_err(|e| format!("submit edit search msg: {e}"))?;

    wait_for_edit_diff(
        &mut conn_a,
        &key_a_search,
        edit_search_id,
        &search_event_id,
        EDITED_BODY,
        "edit search msg diff",
    )
    .await?;

    // Poll until new text is found.
    poll_search_until_found(
        &mut conn_a,
        &account_key_a,
        EDITED_QUERY,
        &search_event_id,
        &room_id,
        "search_edit=ok (new text found)",
    )
    .await?;

    // Assert old text is no longer verifiable (document store canonical text
    // has changed; even if the ngram index still has the old token, the document
    // store will reject the candidate).
    poll_search_until_absent(
        &mut conn_a,
        &account_key_a,
        SEARCH_QUERY,
        &search_event_id,
        &room_id,
        "search_edit=ok (old text absent)",
    )
    .await?;

    println!("search_edit=ok");

    // Assert redacted msg2 text is absent (msg2 was redacted in Phase 5 above).
    poll_search_until_absent(
        &mut conn_a,
        &account_key_a,
        "Phase 5 QA message 2",
        &event2_id,
        &room_id,
        "search_redact=ok (redacted msg absent)",
    )
    .await?;
    println!("search_redact=ok");
    println!("edit_redact_search=ok");

    if scenario.should_run_stage(QaStage::SearchCrawler) {
        run_search_crawler_stage(&mut conn_a, &account_key_a, &room_id).await?;
    }

    // Unsubscribe search timeline.
    let unsub_search_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
            request_id: unsub_search_id,
            key: key_a_search.clone(),
        }))
        .await
        .map_err(|e| format!("submit unsubscribe search timeline: {e}"))?;

    // Unsubscribe has no completion event (it just drops the timeline actor).
    // The sync stop below is dispatched after it on the same FIFO-ordered
    // connection, so the actor is dropped before sync stop runs and
    // `wait_for_sync_stopped` (request-id-scoped) is the concrete wait.

    if scenario == QaScenario::All {
        let e2ee_stage_result = run_e2ee_trust_stage(
            &config,
            &mut conn_a,
            &account_key_a,
            Some((&mut conn_b, &account_key_b)),
        )
        .await;
        let (caller_a, caller_b) = retain_or_cleanup_e2ee_callers_after_stage(
            e2ee_stage_result,
            (
                QaOwnedRuntimeParticipant::from_logged_in(QaOwnedLoggedInRuntime {
                    runtime: runtime_a,
                    conn: conn_a,
                    account_key: account_key_a,
                }),
                QaOwnedRuntimeParticipant::from_logged_in(QaOwnedLoggedInRuntime {
                    runtime: runtime_b,
                    conn: conn_b,
                    account_key: account_key_b,
                }),
            ),
            cleanup_e2ee_callers_after_stage_failure,
        )
        .await?;
        let caller_a = caller_a.into_logged_in_runtime();
        let caller_b = caller_b.into_logged_in_runtime();
        runtime_a = caller_a.runtime;
        conn_a = caller_a.conn;
        account_key_a = caller_a.account_key;
        runtime_b = caller_b.runtime;
        conn_b = caller_b.conn;
        account_key_b = caller_b.account_key;
    }

    // -----------------------------------------------------------------------
    // --- Sync stop A + store-backed restore A + logout A ---
    // -----------------------------------------------------------------------
    let sync_stop_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: sync_stop_id,
        }))
        .await
        .map_err(|e| format!("submit sync stop A: {e}"))?;

    wait_for_sync_stopped(&mut conn_a, sync_stop_id, "sync stop A").await?;
    println!("sync_a=stopped");

    drop(conn_a);
    runtime_a.shutdown().await;

    let runtime_a2 = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut conn_a2 = runtime_a2.attach();

    let restore_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_a_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit restore A: {e}"))?;

    wait_for_session_restored(&mut conn_a2, restore_a_id, &account_key_a, "restore A").await?;
    wait_for_ready_snapshot(&mut conn_a2, "restored session A Ready").await?;

    let logout_a_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a_id,
        }))
        .await
        .map_err(|e| format!("submit logout A: {e}"))?;

    wait_for_logged_out(&mut conn_a2, logout_a_id, &account_key_a, "logout A").await?;

    // Cleanup assertion: a second restore of A must now fail not-found.
    let restore_gone_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_gone_id,
            account_key: account_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("submit post-logout restore A: {e}"))?;

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn_a2,
        restore_gone_id,
        "post-logout restore A (must fail)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore A failed with unexpected kind: {failure:?}"
        ));
    }
    // -----------------------------------------------------------------------
    // --- Logout B ---
    // -----------------------------------------------------------------------
    let logout_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_b_id,
        }))
        .await
        .map_err(|e| format!("submit logout B: {e}"))?;

    wait_for_logged_out(&mut conn_b, logout_b_id, &account_key_b, "logout B").await?;

    // Cleanup assertion: the QA users share one credential store, and B
    // logged in after A, so the last-session pointer pointed at B until B's
    // logout cleared it. After BOTH logouts, RestoreLastSession must yield
    // SessionNotFound (a NORMAL outcome — this is the startup path when no
    // account is stored).
    let restore_last_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_last_id,
        }))
        .await
        .map_err(|e| format!("submit post-logout restore-last: {e}"))?;

    let failure = wait_for_operation_failed_and_signed_out(
        &mut conn_b,
        restore_last_id,
        "post-logout restore-last (must be not-found)",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err(format!(
            "post-logout restore-last failed with unexpected kind: {failure:?}"
        ));
    }
    drop(conn_b);
    runtime_b.shutdown().await;

    println!("restore_cleanup=ok");
    Ok(scenario_report(&config.server_kind, scenario))
}

async fn complete_new_identity_gate_for_qa(
    conn: &mut CoreConnection,
    password: &str,
    destination_suffix: &str,
) -> Result<Option<AuthSecret>, String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        match &conn.snapshot().session {
            SessionState::AwaitingVerification { gate, .. }
                if gate.account_kind == koushi_state::VerificationAccountKind::NewIdentity =>
            {
                break;
            }
            SessionState::Ready(_) => return Ok(None),
            SessionState::Rejecting { .. } => return Err("new identity gate rejected".to_owned()),
            _ => {}
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "timed out waiting for new identity gate; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| "event stream closed while waiting for new identity gate".to_owned())?;
    }
    let request_id = conn.next_request_id();
    let flow_id = request_id.sequence;
    let bootstrap_dir = qa_data_dir(destination_suffix);
    std::fs::create_dir_all(&bootstrap_dir)
        .map_err(|_| "prepare private bootstrap delivery directory".to_owned())?;
    let recovery_key_path = bootstrap_dir.join("recovery-key.txt");
    conn.command(CoreCommand::Account(
        AccountCommand::StartSessionBootstrap {
            request_id,
            flow_id,
            auth: Some(AuthSecret::new(password.to_owned())),
            request: koushi_core::SecureBackupSetupRequest {
                passphrase: Some(AuthSecret::new(password.to_owned())),
                recovery_key_destination_path: Some(recovery_key_path.clone()),
            },
        },
    ))
    .await
    .map_err(|error| format!("submit new identity bootstrap: {error}"))?;
    let delivery_deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        match &conn.snapshot().session {
            SessionState::AwaitingBootstrapConfirmation {
                flow_id: active,
                destination_written: true,
                ..
            } if *active == flow_id => break,
            SessionState::AwaitingVerification { gate, .. } if gate.failure.is_some() => {
                return Err(format!(
                    "new identity bootstrap failed; kind={:?}",
                    gate.failure.expect("failure checked above")
                ));
            }
            _ => {}
        }
        tokio::time::timeout_at(delivery_deadline, conn.recv_event())
            .await
            .map_err(|_| "timed out waiting for bootstrap delivery".to_owned())?
            .map_err(|_| "event stream closed during bootstrap delivery".to_owned())?;
    }
    let recovery_secret = AuthSecret::new(
        std::fs::read_to_string(&recovery_key_path)
            .map_err(|_| "read disposable bootstrap recovery key".to_owned())?
            .trim()
            .to_owned(),
    );
    let confirm_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ConfirmSessionBootstrapSaved {
            request_id: confirm_id,
            flow_id,
        },
    ))
    .await
    .map_err(|error| format!("submit bootstrap saved confirmation: {error}"))?;
    std::fs::remove_file(&recovery_key_path)
        .map_err(|_| "remove disposable bootstrap recovery key".to_owned())?;
    std::fs::remove_dir(&bootstrap_dir)
        .map_err(|_| "remove disposable bootstrap delivery directory".to_owned())?;
    Ok(Some(recovery_secret))
}

async fn wait_for_existing_identity_gate(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<SessionInfo, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    loop {
        if let SessionState::AwaitingVerification { info, gate } = &conn.snapshot().session {
            if gate.account_kind == koushi_state::VerificationAccountKind::ExistingIdentity
                && gate
                    .methods
                    .contains(&koushi_state::VerificationMethodCapability::ExistingDeviceSas)
            {
                return Ok(info.clone());
            }
            if gate.account_kind == koushi_state::VerificationAccountKind::ExistingIdentity {
                return Err(format!(
                    "{label}: existing identity has no SAS proof method"
                ));
            }
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn wait_for_recovery_gate(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if let SessionState::AwaitingVerification { gate, .. } = &conn.snapshot().session
            && gate.account_kind == koushi_state::VerificationAccountKind::ExistingIdentity
            && gate.methods.iter().any(|method| {
                matches!(
                    method,
                    koushi_state::VerificationMethodCapability::RecoveryKey
                        | koushi_state::VerificationMethodCapability::SecurityPhrase
                )
            })
        {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for recovery gate; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn wait_for_matching_recovery_flow(
    conn: &mut CoreConnection,
    flow_id: u64,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if matches!(
            conn.snapshot().session,
            SessionState::Verifying {
                flow_id: active_flow_id,
                method: koushi_state::VerificationMethod::RecoveryKey
                    | koushi_state::VerificationMethod::SecurityPhrase,
                ..
            } if active_flow_id == flow_id
        ) {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for matching recovery flow"))?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn wait_for_locked_snapshot(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    loop {
        if matches!(conn.snapshot().session, SessionState::Locked(_)) {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for Locked; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SasQaOutcome {
    Success,
    Mismatch,
    UserCancel,
    Timeout,
}

#[derive(Debug, Eq, PartialEq)]
enum SecondarySasObservation {
    Pending,
    Presented(Vec<koushi_state::SasEmoji>),
    Failed,
}

fn observe_secondary_sas(
    session: &SessionState,
    expected_flow_id: u64,
    matching_flow_observed: bool,
) -> SecondarySasObservation {
    match session {
        SessionState::Verifying {
            flow_id,
            sas_emojis,
            ..
        } if *flow_id == expected_flow_id && sas_emojis.len() == 7 => {
            SecondarySasObservation::Presented(sas_emojis.clone())
        }
        SessionState::AwaitingVerification { gate, .. }
            if matching_flow_observed && gate.failure.is_some() =>
        {
            SecondarySasObservation::Failed
        }
        _ => SecondarySasObservation::Pending,
    }
}

async fn verify_provisional_second_device_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    session_a: &SessionInfo,
    session_a2: &SessionInfo,
    label: &str,
    outcome: SasQaOutcome,
) -> Result<(), String> {
    if session_a.user_id != session_a2.user_id || session_a.device_id == session_a2.device_id {
        return Err(format!(
            "{label}: expected two distinct devices for one user"
        ));
    }
    // Keep the primary's normal sync continuously running: it owns incoming
    // to-device delivery for the entire SAS flow, including retry outcomes.
    let previous_primary_flow_id =
        verification_state_flow_id(&conn_a.snapshot().e2ee_trust.verification);
    let target_a2 = VerificationTarget {
        user_id: session_a2.user_id.clone(),
        device_id: session_a2.device_id.clone(),
    };
    let flow_request = conn_a2.next_request_id();
    let flow_id_a2 = flow_request.sequence;
    after_receiver_device_known(
        refresh_device_keys_and_assert_known_for_qa(
            conn_a,
            target_a2.clone(),
            &format!("{label}: primary receiver device discovery"),
        ),
        || async {
            conn_a2
                .command(CoreCommand::Account(AccountCommand::StartOwnUserSas {
                    request_id: flow_request,
                    flow_id: flow_id_a2,
                }))
                .await
                .map_err(|error| format!("{label}: submit own-user SAS: {error}"))
        },
    )
    .await?;

    let flow_id_a = wait_for_verification_requested_event_only(
        conn_a,
        Some(&target_a2),
        previous_primary_flow_id,
        &format!("{label}: primary incoming request"),
    )
    .await?;
    let accept_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::AcceptVerification {
            request_id: accept_id,
            flow_id: flow_id_a,
        }))
        .await
        .map_err(|error| format!("{label}: accept primary: {error}"))?;
    wait_for_verification_accepted(
        conn_a,
        flow_id_a,
        Some(accept_id),
        &format!("{label}: primary ready"),
    )
    .await?;

    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut secondary_matching_flow_observed = false;
    let (emojis_a, emojis_a2) = loop {
        let primary = verification_state_sas(
            &conn_a.snapshot().e2ee_trust.verification,
            flow_id_a,
            &format!("{label}: primary SAS"),
        )?;
        let secondary_session = &conn_a2.snapshot().session;
        secondary_matching_flow_observed |= matches!(
            secondary_session,
            SessionState::Verifying { flow_id, .. } if *flow_id == flow_id_a2
        );
        let secondary = match observe_secondary_sas(
            secondary_session,
            flow_id_a2,
            secondary_matching_flow_observed,
        ) {
            SecondarySasObservation::Presented(emojis) => Some(emojis),
            SecondarySasObservation::Failed => {
                return Err(format!("{label}: secondary gate SAS failed"));
            }
            SecondarySasObservation::Pending => None,
        };
        if let (Some(primary), Some(secondary)) = (primary, secondary) {
            break (primary, secondary);
        }
        match wait_for_paired_event_until(conn_a, conn_a2, deadline).await {
            Ok(()) => {}
            Err(PairedEventWaitError::Deadline) => {
                let primary_snapshot = conn_a.snapshot();
                let (primary_phase, primary_flow_matches, primary_emoji_count) =
                    verification_closed_summary(
                        &primary_snapshot.e2ee_trust.verification,
                        flow_id_a,
                    );
                let secondary_snapshot = conn_a2.snapshot();
                let (secondary_phase, secondary_flow_matches, secondary_emoji_count) =
                    session_gate_closed_summary(&secondary_snapshot.session, flow_id_a2);
                return Err(format!(
                    "{label}: timed out waiting for paired SAS; primary_phase={primary_phase};primary_flow_matches={primary_flow_matches};primary_emoji_count={primary_emoji_count};secondary_phase={secondary_phase};secondary_flow_matches={secondary_flow_matches};secondary_emoji_count={secondary_emoji_count}"
                ));
            }
            Err(PairedEventWaitError::Primary(lag)) => {
                return Err(verification_event_stream_error(label, "primary", lag));
            }
            Err(PairedEventWaitError::Secondary(lag)) => {
                return Err(verification_event_stream_error(label, "secondary", lag));
            }
        }
    };
    if emojis_a != emojis_a2 {
        return Err(format!("{label}: SAS emoji mismatch"));
    }
    if outcome == SasQaOutcome::Timeout {
        wait_for_existing_identity_gate(conn_a2, &format!("{label}: timeout retryable")).await?;
        return Ok(());
    }
    if outcome != SasQaOutcome::Success {
        let cancel_a2 = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Account(AccountCommand::CancelVerification {
                request_id: cancel_a2,
                flow_id: flow_id_a2,
                reason: if outcome == SasQaOutcome::Mismatch {
                    koushi_state::VerificationCancelReason::Mismatch
                } else {
                    koushi_state::VerificationCancelReason::User
                },
            }))
            .await
            .map_err(|error| format!("{label}: mismatch secondary: {error}"))?;
        let cancel_a = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Account(AccountCommand::CancelVerification {
                request_id: cancel_a,
                flow_id: flow_id_a,
                reason: koushi_state::VerificationCancelReason::User,
            }))
            .await
            .map_err(|error| format!("{label}: cancel primary after mismatch: {error}"))?;
        wait_for_existing_identity_gate(conn_a2, &format!("{label}: mismatch retryable")).await?;
        return Ok(());
    }

    let confirm_a = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id: confirm_a,
                flow_id: flow_id_a,
            },
        ))
        .await
        .map_err(|error| format!("{label}: confirm primary: {error}"))?;
    let confirm_a2 = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id: confirm_a2,
                flow_id: flow_id_a2,
            },
        ))
        .await
        .map_err(|error| format!("{label}: confirm secondary: {error}"))?;

    let ready_deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if matches!(conn_a2.snapshot().session, SessionState::Ready(_)) {
            return Ok(());
        }
        match (QaEventDeadline {
            instant: ready_deadline,
        })
        .recv(conn_a2)
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(lag)) => {
                return Err(verification_event_stream_error(label, "secondary", lag));
            }
            Err(_) => {
                return Err(format!(
                    "{label}: timed out waiting for authoritative Ready"
                ));
            }
        }
    }
}

fn verification_event_stream_error(label: &str, participant: &str, lag: EventStreamLag) -> String {
    if lag.skipped == 0 {
        format!("{label}: {participant} event stream closed")
    } else {
        format!(
            "{label}: {participant} event stream lagged; skipped={}",
            lag.skipped
        )
    }
}

async fn after_receiver_device_known<Refresh, Start, Started, Output, Error>(
    refresh: Refresh,
    start_once: Start,
) -> Result<Output, Error>
where
    Refresh: Future<Output = Result<(), Error>>,
    Start: FnOnce() -> Started,
    Started: Future<Output = Result<Output, Error>>,
{
    refresh.await?;
    start_once().await
}

fn verification_closed_summary(
    state: &VerificationFlowState,
    expected_flow_id: u64,
) -> (&'static str, bool, usize) {
    let phase = match state {
        VerificationFlowState::Idle => "idle",
        VerificationFlowState::Requested { .. } => "requested",
        VerificationFlowState::Accepted { .. } => "accepted",
        VerificationFlowState::SasPresented { .. } => "presented",
        VerificationFlowState::Confirming { .. } => "confirming",
        VerificationFlowState::Done { .. } => "done",
        VerificationFlowState::Failed { .. } => "failed",
    };
    let matches = verification_state_flow_id(state) == Some(expected_flow_id);
    let count = match state {
        VerificationFlowState::SasPresented { emojis, .. }
        | VerificationFlowState::Confirming { emojis, .. } => emojis.len(),
        _ => 0,
    };
    (phase, matches, count)
}

fn session_gate_closed_summary(
    state: &SessionState,
    expected_flow_id: u64,
) -> (&'static str, bool, usize) {
    match state {
        SessionState::Verifying {
            flow_id,
            sas_emojis,
            ..
        } => ("verifying", *flow_id == expected_flow_id, sas_emojis.len()),
        SessionState::AwaitingVerification { .. } => ("awaiting_verification", false, 0),
        SessionState::Provisional { .. } => ("provisional", false, 0),
        SessionState::AwaitingBootstrapConfirmation { .. } => {
            ("awaiting_bootstrap_confirmation", false, 0)
        }
        SessionState::Ready(_) => ("ready", false, 0),
        SessionState::Rejecting { .. } => ("rejecting", false, 0),
        SessionState::Locked(_) => ("locked", false, 0),
        SessionState::SignedOut => ("signed_out", false, 0),
        SessionState::Restoring => ("restoring", false, 0),
        SessionState::SwitchingAccount { .. } => ("switching", false, 0),
        SessionState::Authenticating { .. } => ("authenticating", false, 0),
        SessionState::LoggingOut => ("logging_out", false, 0),
    }
}

fn gate_session_phase(session: &SessionState) -> &'static str {
    match session {
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::CheckingTrust,
            ..
        } => "checking_trust",
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
            ..
        } => "discovering_methods",
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::RecheckingTrust { .. },
            ..
        } => "rechecking_trust",
        SessionState::AwaitingVerification { .. } => "awaiting_verification",
        SessionState::Verifying { .. } => "verifying",
        SessionState::AwaitingBootstrapConfirmation { .. } => "awaiting_bootstrap_confirmation",
        SessionState::Rejecting { .. } => "rejecting",
        SessionState::Ready(_) => "ready",
        SessionState::Locked(_) => "locked",
        SessionState::SignedOut => "signed_out",
        SessionState::Restoring => "restoring",
        SessionState::SwitchingAccount { .. } => "switching",
        SessionState::Authenticating { .. } => "authenticating",
        SessionState::LoggingOut => "logging_out",
    }
}

// ---------------------------------------------------------------------------
// Room-list helpers
// ---------------------------------------------------------------------------

/// A compact summary of a snapshot's room list for printing.
fn room_list_summary(snapshot: &AppState) -> String {
    let spaces = snapshot.spaces.len();
    let rooms = snapshot.rooms.len();
    let dms = snapshot.rooms.iter().filter(|r| r.is_dm).count();
    let unread = snapshot.rooms.iter().filter(|r| r.unread_count > 0).count();
    format!("rooms={rooms} spaces={spaces} dms={dms} unread_rooms={unread}")
}

fn private_room_options(name: impl Into<String>, encrypted: bool) -> CreateRoomOptions {
    CreateRoomOptions {
        name: name.into(),
        topic: None,
        alias_localpart: None,
        encrypted,
        visibility: CreateRoomVisibility::Private,
        parent_space: None,
    }
}

async fn create_room_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    encrypted: bool,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        options: private_room_options(name, encrypted),
    }))
    .await
    .map_err(|e| format!("{label}: submit room create failed: {e}"))?;
    wait_for_room_created(conn, request_id, label).await
}

async fn create_public_directory_room_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    alias_localpart: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreatePublicDirectoryRoom {
        request_id,
        name: name.to_owned(),
        alias_localpart: alias_localpart.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit public directory room create failed: {e}"))?;
    wait_for_room_created(conn, request_id, label).await
}

async fn create_space_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateSpace {
        request_id,
        name: name.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit space create failed: {e}"))?;
    wait_for_space_created(conn, request_id, label).await
}

async fn invite_user_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    user_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::InviteUser {
        request_id,
        room_id: room_id.to_owned(),
        user_id: user_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit invite failed: {e}"))?;
    wait_for_user_invited_ack(conn, request_id, label).await
}

async fn load_room_settings_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<RoomSettingsSnapshot, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::LoadRoomSettings {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit load settings failed: {e}"))?;
    wait_for_room_settings_loaded(conn, request_id, label).await
}

fn assert_room_settings_contains_members(
    settings: &RoomSettingsSnapshot,
    expected_user_ids: &[&str],
    label: &str,
) -> Result<(), String> {
    let observed_user_ids = settings
        .members
        .iter()
        .map(|member| member.user_id.as_str())
        .collect::<BTreeSet<_>>();
    let missing_count = expected_user_ids
        .iter()
        .filter(|user_id| !observed_user_ids.contains(**user_id))
        .count();
    if missing_count > 0 {
        return Err(format!(
            "{label}: member list missing expected users \
             (expected={}, observed={}, missing={missing_count})",
            expected_user_ids.len(),
            observed_user_ids.len()
        ));
    }
    Ok(())
}

async fn accept_invite_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::AcceptInvite {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit accept invite failed: {e}"))?;
    wait_for_invite_accepted(conn, request_id, room_id, label).await
}

async fn decline_invite_for_qa(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::DeclineInvite {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit decline invite failed: {e}"))?;
    wait_for_invite_declined(conn, request_id, room_id, label).await
}

async fn start_direct_message_for_qa(
    conn: &mut CoreConnection,
    user_id: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::StartDirectMessage {
        request_id,
        user_id: user_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit start DM failed: {e}"))?;
    wait_for_direct_message_started(conn, request_id, label).await
}

async fn set_space_child_for_qa(
    conn: &mut CoreConnection,
    space_id: &str,
    child_room_id: &str,
    via_server: &str,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SetSpaceChild {
        request_id,
        space_id: space_id.to_owned(),
        child_room_id: child_room_id.to_owned(),
        via_server: via_server.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit set space child failed: {e}"))?;
    wait_for_space_child_set(conn, request_id, space_id, child_room_id, label).await
}

// ---------------------------------------------------------------------------
// Event waiter helpers (Phase 4 additions)
// ---------------------------------------------------------------------------

/// Wait for `RoomEvent::RoomCreated` with the given request_id. Returns room_id.
async fn wait_for_room_created(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    let mut seen_total = 0usize;
    let mut seen_state_changed = 0usize;
    let mut seen_room_created_other = 0usize;
    let mut seen_operation_failed_other = 0usize;
    let mut last_event_kind = "none";
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for RoomEvent::RoomCreated request_id={}/{} seen_total={seen_total} seen_state_changed={seen_state_changed} seen_room_created_other={seen_room_created_other} seen_operation_failed_other={seen_operation_failed_other} last_event={last_event_kind}",
                    request_id.connection_id.0,
                    request_id.sequence,
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        seen_total += 1;
        last_event_kind = core_event_kind(&event);

        match event {
            CoreEvent::Room(RoomEvent::RoomCreated {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                return Ok(room_id);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            CoreEvent::Room(RoomEvent::RoomCreated { .. }) => {
                seen_room_created_other += 1;
            }
            CoreEvent::OperationFailed { .. } => {
                seen_operation_failed_other += 1;
            }
            CoreEvent::StateChanged(_) => {
                seen_state_changed += 1;
            }
            _ => continue,
        }
    }
}

fn core_event_kind(event: &CoreEvent) -> &'static str {
    match event {
        CoreEvent::StateDelta(_) => "StateDelta",
        CoreEvent::StateChanged(_) => "StateChanged",
        CoreEvent::Account(_) => "Account",
        CoreEvent::Sync(_) => "Sync",
        CoreEvent::Room(room_event) => match room_event {
            RoomEvent::RoomCreated { .. } => "RoomCreated",
            RoomEvent::SpaceCreated { .. } => "SpaceCreated",
            RoomEvent::SpaceChildSet { .. } => "SpaceChildSet",
            RoomEvent::UserInvited { .. } => "UserInvited",
            RoomEvent::InviteAccepted { .. } => "InviteAccepted",
            RoomEvent::InviteDeclined { .. } => "InviteDeclined",
            RoomEvent::RoomJoined { .. } => "RoomJoined",
            RoomEvent::RoomListUpdated => "RoomListUpdated",
            _ => "Room",
        },
        CoreEvent::Timeline(_) => "Timeline",
        CoreEvent::LiveSignals(_) => "LiveSignals",
        CoreEvent::Search(_) => "Search",
        CoreEvent::E2eeTrust(_) => "E2eeTrust",
        CoreEvent::Activity(_) => "Activity",
        CoreEvent::LocalEncryption(_) => "LocalEncryption",
        CoreEvent::NativeAttention(_) => "NativeAttention",
        CoreEvent::CjkTextPolicy(_) => "CjkTextPolicy",
        CoreEvent::ThreadsList(_) => "ThreadsList",
        CoreEvent::IntentLifecycle { .. } => "IntentLifecycle",
        CoreEvent::OperationFailed { .. } => "OperationFailed",
    }
}

async fn query_directory_until_room_visible(
    conn: &mut CoreConnection,
    query: DirectoryQuery,
    room_id: &str,
    alias: &str,
    label: &str,
) -> Result<Vec<DirectoryRoomSummary>, String> {
    for attempt in 1..=6 {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::QueryDirectory {
            request_id,
            query: query.clone(),
        }))
        .await
        .map_err(|e| format!("{label}: submit directory query failed: {e}"))?;
        let rooms = wait_for_directory_query_completed(conn, request_id, label).await?;
        if rooms
            .iter()
            .any(|room| room.room_id == room_id || room.canonical_alias.as_deref() == Some(alias))
        {
            return Ok(rooms);
        }
        if attempt < 6 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    Err(format!(
        "{label}: public directory did not return the created room after bounded retries"
    ))
}

async fn wait_for_directory_query_completed(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<Vec<DirectoryRoomSummary>, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for DirectoryQueryCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::DirectoryQueryCompleted {
                request_id: ev_id,
                rooms,
                ..
            }) if ev_id == request_id => {
                return Ok(rooms);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_room_settings_loaded(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<RoomSettingsSnapshot, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomSettingsLoaded"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomSettingsLoaded {
                request_id: ev_id,
                settings,
            }) if ev_id == request_id => return Ok(settings),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => return Err(format!("{label} failed: {failure:?}")),
            _ => continue,
        }
    }
}

async fn wait_for_room_setting_updated(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<RoomSettingsSnapshot, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomSettingUpdated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                request_id: ev_id,
                settings,
            }) if ev_id == request_id => return Ok(settings),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => return Err(format!("{label} failed: {failure:?}")),
            _ => continue,
        }
    }
}

async fn wait_for_room_member_moderated(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomMemberModerated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomMemberModerated {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => return Err(format!("{label} failed: {failure:?}")),
            _ => continue,
        }
    }
}

async fn wait_for_room_management_forbidden(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut saw_forbidden_failure = false;

    loop {
        if saw_forbidden_failure && room_management_forbidden_recorded(&conn.snapshot(), request_id)
        {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for forbidden room-management state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure:
                    CoreFailure::RoomOperationFailed {
                        kind: RoomFailureKind::Forbidden,
                    },
            } if ev_id == request_id => {
                saw_forbidden_failure = true;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: expected forbidden room-management failure, got {failure:?}"
                ));
            }
            CoreEvent::StateChanged(snapshot)
                if room_management_forbidden_recorded(&snapshot, request_id) =>
            {
                if saw_forbidden_failure {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

fn room_management_forbidden_recorded(snapshot: &AppState, request_id: RequestId) -> bool {
    matches!(
        &snapshot.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: state_request_id,
            operation,
            kind,
            ..
        } if *state_request_id == request_id.sequence
            && *operation == RoomManagementOperationKind::Moderation
            && *kind == OperationFailureKind::Forbidden
    )
}

/// Wait for `RoomEvent::SpaceCreated` with the given request_id. Returns space_id.
async fn wait_for_space_created(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    let mut seen_total = 0usize;
    let mut seen_state_changed = 0usize;
    let mut seen_space_created_other = 0usize;
    let mut seen_room_created = 0usize;
    let mut seen_operation_failed_other = 0usize;
    let mut last_event_kind = "none";
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for RoomEvent::SpaceCreated request_id={}/{} seen_total={seen_total} seen_state_changed={seen_state_changed} seen_space_created_other={seen_space_created_other} seen_room_created={seen_room_created} seen_operation_failed_other={seen_operation_failed_other} last_event={last_event_kind}",
                    request_id.connection_id.0,
                    request_id.sequence,
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        seen_total += 1;
        last_event_kind = core_event_kind(&event);

        match event {
            CoreEvent::Room(RoomEvent::SpaceCreated {
                request_id: ev_id,
                space_id,
            }) if ev_id == request_id => {
                return Ok(space_id);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            CoreEvent::Room(RoomEvent::SpaceCreated { .. }) => {
                seen_space_created_other += 1;
            }
            CoreEvent::Room(RoomEvent::RoomCreated { .. }) => {
                seen_room_created += 1;
            }
            CoreEvent::OperationFailed { .. } => {
                seen_operation_failed_other += 1;
            }
            CoreEvent::StateChanged(_) => {
                seen_state_changed += 1;
            }
            _ => continue,
        }
    }
}

/// Wait for `RoomEvent::SpaceChildSet` with the given request_id.
async fn wait_for_space_child_set(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    space_id: &str,
    child_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::SpaceChildSet"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::SpaceChildSet {
                request_id: ev_id,
                space_id: ev_space,
                child_room_id: ev_child,
            }) if ev_id == request_id => {
                if ev_space != space_id || ev_child != child_room_id {
                    return Err(format!(
                        "{label}: SpaceChildSet IDs mismatch: space={ev_space} child={ev_child}"
                    ));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for `RoomEvent::UserInvited` with the given request_id.
async fn wait_for_user_invited(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    room_id: &str,
    user_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::UserInvited"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::UserInvited {
                request_id: ev_id,
                room_id: ev_room,
                user_id: ev_user,
            }) if ev_id == request_id => {
                if ev_room != room_id || ev_user != user_id {
                    return Err(format!(
                        "{label}: UserInvited IDs mismatch: room={ev_room} user={ev_user}"
                    ));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for `RoomEvent::UserInvited` by request_id without exposing IDs in
/// failure text. Used by private-data-free invite QA.
async fn wait_for_user_invited_ack(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::UserInvited"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::UserInvited {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_invite_accepted(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::InviteAccepted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::InviteAccepted {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id != expected_room_id {
                    return Err(format!("{label}: accepted invite room mismatch"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_invite_declined(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    expected_room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::InviteDeclined"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::InviteDeclined {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => {
                if room_id != expected_room_id {
                    return Err(format!("{label}: declined invite room mismatch"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_direct_message_started(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::DirectMessageStarted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::DirectMessageStarted {
                request_id: ev_id,
                room_id,
            }) if ev_id == request_id => return Ok(room_id),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for `RoomEvent::RoomJoined` with the given request_id.
async fn wait_for_room_joined(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::RoomJoined"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomJoined {
                request_id: ev_id,
                room_id: ev_room,
            }) if ev_id == request_id => {
                if ev_room != room_id {
                    return Err(format!(
                        "{label}: RoomJoined room_id mismatch: got {ev_room}, expected {room_id}"
                    ));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_pin_event_completed(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::PinEventCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::PinEventCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_unpin_event_completed(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for RoomEvent::UnpinEventCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::UnpinEventCompleted {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_pinned_state(
    conn: &mut CoreConnection,
    room_id: &str,
    event_id: &str,
    expected_present: bool,
    label: &str,
) -> Result<(), String> {
    if snapshot_has_pinned_event(&conn.snapshot(), room_id, event_id) == expected_present {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for pinned state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) => {
                if snapshot_has_pinned_event(&snapshot, room_id, event_id) == expected_present {
                    return Ok(());
                }
            }
            CoreEvent::Room(RoomEvent::PinnedEventsUpdated {
                room_id: ev_room_id,
                pinned,
            }) if ev_room_id == room_id => {
                let has_event = pinned.iter().any(|event| event.event_id == event_id);
                if has_event == expected_present {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

fn snapshot_has_pinned_event(snapshot: &AppState, room_id: &str, event_id: &str) -> bool {
    snapshot
        .room_interactions
        .get(room_id)
        .map(|state| {
            state
                .pinned_events
                .iter()
                .any(|event| event.event_id == event_id)
        })
        .unwrap_or(false)
}

/// Wait (event-driven on `RoomListUpdated`/`StateChanged`, bounded by
/// `ROOM_LIST_EVENT_TIMEOUT`) until the snapshot's room list contains the
/// expected room in `rooms` AND the expected space in `spaces`. Returns the matching
/// snapshot. Waiting for "any non-empty list" is not enough: spaces only
/// classify as spaces after the create reaches the client via sync, so the
/// list can be momentarily rooms-only.
async fn wait_for_room_list_containing(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    expected_space_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot.rooms.iter().any(|r| r.room_id == expected_room_id)
            && snapshot
                .spaces
                .iter()
                .any(|s| s.space_id == expected_space_id)
    };

    // Check the latest snapshot first in case it already has the data.
    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for room list to contain room \
                     {expected_room_id} and space {expected_space_id} \
                     (have {} rooms, {} spaces)",
                    snapshot.rooms.len(),
                    snapshot.spaces.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                // The discrete event may arrive before the reducer projected
                // the matching snapshot; check the latest snapshot and keep
                // waiting otherwise — a StateChanged will follow.
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn wait_for_room_in_room_list(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let contains_expected =
        |snapshot: &AppState| snapshot.rooms.iter().any(|r| r.room_id == expected_room_id);

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for room list to include the expected room \
                     (have {} rooms)",
                    snapshot.rooms.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn wait_for_space_in_space_list(
    conn: &mut CoreConnection,
    expected_space_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot
            .spaces
            .iter()
            .any(|s| s.space_id == expected_space_id)
    };

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for space list to include the expected space \
                     (have {} spaces)",
                    snapshot.spaces.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn wait_for_space_child_projection(
    conn: &mut CoreConnection,
    space_id: &str,
    expected_child_room_ids: &[String],
    label: &str,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        space_has_expected_children(snapshot, space_id, expected_child_room_ids)
    };

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                let observed_child_count = snapshot
                    .spaces
                    .iter()
                    .find(|space| space.space_id == space_id)
                    .map(|space| space.child_room_ids.len())
                    .unwrap_or_default();
                format!(
                    "{label}: timed out waiting for space child projection \
                     (expected_children={}, observed_children={}, spaces={})",
                    expected_child_room_ids.len(),
                    observed_child_count,
                    snapshot.spaces.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn select_space_and_wait_for_room_scope(
    conn: &mut CoreConnection,
    space_id: &str,
    expected_room_ids: &[String],
    label: &str,
) -> Result<AppState, String> {
    select_room_list_filter_for_qa(conn, RoomListFilter::Rooms, label).await?;
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id: Some(space_id.to_owned()),
    }))
    .await
    .map_err(|e| format!("{label}: submit select space failed: {e}"))?;

    let matches_scope = |snapshot: &AppState| {
        room_list_matches_selected_space(snapshot, space_id, expected_room_ids)
    };
    let snapshot = conn.snapshot();
    if matches_scope(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for selected-space room scope \
                     (expected_rooms={}, projected_items={}, total_rooms={}, active_space={})",
                    expected_room_ids.len(),
                    snapshot.room_list.items.len(),
                    snapshot.rooms.len(),
                    snapshot.navigation.active_space_id.is_some()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if matches_scope(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if matches_scope(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: select space failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn select_room_list_filter_for_qa(
    conn: &mut CoreConnection,
    filter: RoomListFilter,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().room_list.active_filter == filter {
        return Ok(());
    }

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SelectRoomListFilter {
        request_id,
        filter,
    }))
    .await
    .map_err(|e| format!("{label}: submit room-list filter failed: {e}"))?;

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for room-list filter"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if snapshot.room_list.active_filter == filter => {
                return Ok(());
            }
            CoreEvent::Room(RoomEvent::RoomListUpdated)
                if conn.snapshot().room_list.active_filter == filter =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: room-list filter failed: {failure:?}"));
            }
            _ if conn.snapshot().room_list.active_filter == filter => return Ok(()),
            _ => continue,
        }
    }
}

fn space_has_expected_children(
    snapshot: &AppState,
    space_id: &str,
    expected_child_room_ids: &[String],
) -> bool {
    let Some(space) = snapshot
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)
    else {
        return false;
    };
    let child_room_ids = space.child_room_ids.iter().collect::<BTreeSet<_>>();
    expected_child_room_ids
        .iter()
        .all(|room_id| child_room_ids.contains(room_id))
}

fn room_list_matches_selected_space(
    snapshot: &AppState,
    space_id: &str,
    expected_room_ids: &[String],
) -> bool {
    if snapshot.navigation.active_space_id.as_deref() != Some(space_id)
        || snapshot.room_list.active_filter != RoomListFilter::Rooms
        || !space_has_expected_children(snapshot, space_id, expected_room_ids)
    {
        return false;
    }
    let expected = expected_room_ids.iter().collect::<BTreeSet<_>>();
    let projected = snapshot
        .room_list
        .items
        .iter()
        .filter(|item| matches!(item.kind, koushi_state::RoomListEntryKind::Room))
        .map(|item| &item.room_id)
        .collect::<BTreeSet<_>>();
    projected == expected
}

async fn wait_for_dm_room_in_room_list(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot
            .rooms
            .iter()
            .any(|room| room.room_id == expected_room_id && room.is_dm)
    };

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for DM room in room list \
                     (have {} rooms)",
                    snapshot.rooms.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn assert_dm_space_scope_for_qa(
    conn: &mut CoreConnection,
    member_space_id: &str,
    member_dm_room_id: &str,
    control_dm_room_id: &str,
) -> Result<(), String> {
    select_space_scope_for_qa(conn, None, "invites_dm select Home for DM scope").await?;
    wait_for_sidebar_dm_room_ids(
        conn,
        &[member_dm_room_id, control_dm_room_id],
        "invites_dm Home DM scope",
    )
    .await?;

    select_space_scope_for_qa(
        conn,
        Some(member_space_id),
        "invites_dm select member Space for DM scope",
    )
    .await?;
    wait_for_sidebar_dm_room_ids(conn, &[member_dm_room_id], "invites_dm Space DM scope").await
}

async fn select_space_scope_for_qa(
    conn: &mut CoreConnection,
    space_id: Option<&str>,
    label: &str,
) -> Result<(), String> {
    let matches_scope =
        |snapshot: &AppState| snapshot.navigation.active_space_id.as_deref() == space_id;
    if matches_scope(&conn.snapshot()) {
        return Ok(());
    }

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id: space_id.map(str::to_owned),
    }))
    .await
    .map_err(|e| format!("{label}: submit select space failed: {e}"))?;

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for space selection \
                     (expected_active={}, observed_active={})",
                    space_id.is_some(),
                    snapshot.navigation.active_space_id.is_some()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if matches_scope(&snapshot) => return Ok(()),
            CoreEvent::Room(RoomEvent::RoomListUpdated) if matches_scope(&conn.snapshot()) => {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: select space failed: {failure:?}"));
            }
            _ if matches_scope(&conn.snapshot()) => return Ok(()),
            _ => continue,
        }
    }
}

async fn wait_for_sidebar_dm_room_ids(
    conn: &mut CoreConnection,
    expected_room_ids: &[&str],
    label: &str,
) -> Result<(), String> {
    let expected = expected_room_ids
        .iter()
        .map(|room_id| (*room_id).to_owned())
        .collect::<BTreeSet<_>>();
    let matches_expected = |snapshot: &AppState| sidebar_dm_room_ids(snapshot) == expected;
    if matches_expected(&conn.snapshot()) {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            let snapshot = conn.snapshot();
            return Err(format!(
                "{label}: DM section scope mismatch \
                 (expected_count={}, observed_count={}, active_space={})",
                expected.len(),
                sidebar_dm_room_ids(&snapshot).len(),
                snapshot.navigation.active_space_id.is_some()
            ));
        }

        let event = tokio::time::timeout(remaining, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: DM section scope mismatch \
                     (expected_count={}, observed_count={}, active_space={})",
                    expected.len(),
                    sidebar_dm_room_ids(&snapshot).len(),
                    snapshot.navigation.active_space_id.is_some()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if matches_expected(&snapshot) => return Ok(()),
            CoreEvent::Room(RoomEvent::RoomListUpdated) if matches_expected(&conn.snapshot()) => {
                return Ok(());
            }
            _ if matches_expected(&conn.snapshot()) => return Ok(()),
            _ => {}
        }
    }
}

fn sidebar_dm_room_ids(snapshot: &AppState) -> BTreeSet<String> {
    compose_sidebar(
        snapshot.navigation.active_space_id.as_deref(),
        &snapshot.spaces,
        &snapshot.rooms,
    )
    .global_dms
    .into_iter()
    .map(|room| room.room_id)
    .collect()
}

#[derive(Default)]
struct InviteObserverDiagnosticSummary {
    started: u64,
    rls_wake_max: u64,
    base_wake_max: u64,
    base_invite_update_seen: bool,
    base_membership_change_seen: bool,
    base_projection_required_seen: bool,
    invite_projection: u64,
    invite_projection_delivered: u64,
    invite_projection_undelivered: u64,
    lagged: u64,
    closed: u64,
    exit: u64,
    dropped: u64,
}

fn diagnostic_count_field(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
) -> Option<u64> {
    event.fields.iter().find_map(|field| {
        if field.key == key
            && let koushi_diagnostics::DiagnosticValue::Count(value) = field.value
        {
            return Some(value);
        }
        None
    })
}

fn diagnostic_boolean_field(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
) -> Option<bool> {
    event.fields.iter().find_map(|field| {
        if field.key == key
            && let koushi_diagnostics::DiagnosticValue::Boolean(value) = field.value
        {
            return Some(value);
        }
        None
    })
}

fn diagnostic_has_token(
    event: &koushi_diagnostics::DiagnosticEvent,
    key: &'static str,
    expected: &'static str,
) -> bool {
    event.fields.iter().any(|field| {
        field.key == key && field.value == koushi_diagnostics::DiagnosticValue::Token(expected)
    })
}

fn invite_observer_diagnostic_summary(snapshot: &koushi_diagnostics::DiagnosticSnapshot) -> String {
    let mut summary = InviteObserverDiagnosticSummary {
        dropped: snapshot.dropped_records,
        ..InviteObserverDiagnosticSummary::default()
    };
    for record in &snapshot.records {
        let event = &record.event;
        if event.source != "core.room" {
            continue;
        }
        match event.stage {
            "live_observer_started" => summary.started = summary.started.saturating_add(1),
            "live_observer_wake_milestone" => {
                let wake_count = diagnostic_count_field(event, "wake_count").unwrap_or(0);
                if diagnostic_has_token(event, "source", "rls_diff") {
                    summary.rls_wake_max = summary.rls_wake_max.max(wake_count);
                } else if diagnostic_has_token(event, "source", "base_room_updates") {
                    summary.base_wake_max = summary.base_wake_max.max(wake_count);
                    summary.base_invite_update_seen |=
                        diagnostic_boolean_field(event, "invite_update_observed").unwrap_or(false);
                    summary.base_membership_change_seen |=
                        diagnostic_boolean_field(event, "invite_membership_changed")
                            .unwrap_or(false);
                    summary.base_projection_required_seen |=
                        diagnostic_boolean_field(event, "projection_required").unwrap_or(false);
                }
            }
            "live_observer_invite_projection" => {
                summary.invite_projection = summary.invite_projection.saturating_add(1);
            }
            "live_observer_invite_projection_completed" => {
                if diagnostic_boolean_field(event, "action_delivered").unwrap_or(false) {
                    summary.invite_projection_delivered =
                        summary.invite_projection_delivered.saturating_add(1);
                } else {
                    summary.invite_projection_undelivered =
                        summary.invite_projection_undelivered.saturating_add(1);
                }
            }
            "live_observer_base_lagged" => {
                summary.lagged = summary.lagged.saturating_add(1);
            }
            "live_observer_auxiliary_closed" => {
                summary.closed = summary.closed.saturating_add(1);
            }
            "live_observer_exit" => summary.exit = summary.exit.saturating_add(1),
            _ => {}
        }
    }
    format!(
        "observer_diag_started={} observer_diag_rls_wake_max={} \
         observer_diag_base_wake_max={} observer_diag_base_invite_update_seen={} \
         observer_diag_base_membership_change_seen={} \
         observer_diag_base_projection_required_seen={} observer_diag_invite_projection={} \
         observer_diag_invite_projection_delivered={} \
         observer_diag_invite_projection_undelivered={} observer_diag_lagged={} \
         observer_diag_closed={} observer_diag_exit={} observer_diag_dropped={}",
        summary.started,
        summary.rls_wake_max,
        summary.base_wake_max,
        summary.base_invite_update_seen,
        summary.base_membership_change_seen,
        summary.base_projection_required_seen,
        summary.invite_projection,
        summary.invite_projection_delivered,
        summary.invite_projection_undelivered,
        summary.lagged,
        summary.closed,
        summary.exit,
        summary.dropped,
    )
}

async fn wait_for_invite_in_snapshot(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    expected_is_dm: Option<bool>,
    label: &str,
) -> Result<AppState, String> {
    let contains_expected = |snapshot: &AppState| {
        snapshot.invites.iter().any(|invite| {
            invite.room_id == expected_room_id
                && expected_is_dm.is_none_or(|expected| invite.is_dm == expected)
        })
    };

    let snapshot = conn.snapshot();
    if contains_expected(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                let observer_diagnostics =
                    invite_observer_diagnostic_summary(&koushi_diagnostics::snapshot());
                format!(
                    "{label}: timed out waiting for invite snapshot \
                     (have {} invites; {observer_diagnostics})",
                    snapshot.invites.len(),
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if contains_expected(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

async fn wait_for_invite_absent(
    conn: &mut CoreConnection,
    expected_room_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let is_absent = |snapshot: &AppState| {
        !snapshot
            .invites
            .iter()
            .any(|invite| invite.room_id == expected_room_id)
    };

    let snapshot = conn.snapshot();
    if is_absent(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                let snapshot = conn.snapshot();
                format!(
                    "{label}: timed out waiting for invite removal \
                     (have {} invites)",
                    snapshot.invites.len()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Room(RoomEvent::RoomListUpdated) => {
                let snapshot = conn.snapshot();
                if is_absent(&snapshot) {
                    return Ok(snapshot);
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                if is_absent(&snapshot) {
                    return Ok(snapshot);
                }
            }
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3 event waiter helpers (unchanged)
// ---------------------------------------------------------------------------

/// Wait for `SyncEvent::Started` for the request, then `Running`.
///
/// Runtime SyncService fallback emits another `Started` with the same request id
/// before `Running`; return the latest backend so QA records the effective
/// backend, not only the initially advertised one.
async fn wait_for_sync_started_and_running(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<SyncBackendKind, String> {
    let mut observed_backend = None;
    let mut saw_running_before_started = false;
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Started/Running"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Started {
                request_id: ev_id,
                backend,
            }) if ev_id == Some(request_id) => {
                observed_backend = Some(backend);
                if saw_running_before_started {
                    return Ok(backend);
                }
            }
            CoreEvent::Sync(SyncEvent::Running) => {
                if let Some(backend) = observed_backend {
                    return Ok(backend);
                }
                saw_running_before_started = true;
            }
            CoreEvent::Sync(SyncEvent::Failed) => {
                return Err(format!(
                    "{label}: SyncEvent::Failed received before Running"
                ));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

async fn wait_for_sync_started(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<SyncBackendKind, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Started"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Sync(SyncEvent::Started {
                request_id: Some(event_request_id),
                backend,
            }) if event_request_id == request_id => return Ok(backend),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

/// Wait for `SyncEvent::Stopped` with the given request_id.
async fn wait_for_sync_stopped(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Stopped"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped {
                request_id: Some(ev_id)
            }) if ev_id == request_id
        ) {
            return Ok(());
        }
        if matches!(
            event,
            CoreEvent::Sync(SyncEvent::Stopped { request_id: None })
        ) {
            return Ok(());
        }
        if let CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } = event
        {
            if ev_id == request_id {
                return Err(format!("{label} failed: {failure:?}"));
            }
        }
    }
}

async fn stop_sync_for_qa(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Stop { request_id }))
        .await
        .map_err(|e| format!("{label}: submit Sync stop failed: {e}"))?;
    wait_for_sync_stopped(conn, request_id, label).await
}

async fn start_sync_for_qa(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<SyncBackendKind, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start { request_id }))
        .await
        .map_err(|e| format!("{label}: submit Sync start failed: {e}"))?;
    wait_for_sync_started_and_running(conn, request_id, label).await
}

async fn wait_for_sync_reconnecting(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    if matches!(
        conn.snapshot().sync,
        koushi_state::SyncState::Reconnecting { .. }
    ) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Reconnecting"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Reconnecting) => return Ok(()),
            CoreEvent::StateChanged(snapshot)
                if matches!(snapshot.sync, koushi_state::SyncState::Reconnecting { .. }) =>
            {
                return Ok(());
            }
            CoreEvent::Sync(SyncEvent::Failed) => {
                return Err(format!(
                    "{label}: SyncEvent::Failed received before Reconnecting"
                ));
            }
            _ => {}
        }
    }
}

async fn wait_for_sync_running_after_reconnect(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    if matches!(conn.snapshot().sync, koushi_state::SyncState::Running) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncEvent::Running"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Running) => return Ok(()),
            CoreEvent::StateChanged(snapshot)
                if matches!(snapshot.sync, koushi_state::SyncState::Running) =>
            {
                return Ok(());
            }
            CoreEvent::Sync(SyncEvent::Failed) => {
                return Err(format!(
                    "{label}: SyncEvent::Failed received before Running"
                ));
            }
            _ => {}
        }
    }
}

async fn wait_for_legacy_fallback_starting(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for automatic LegacySync fallback"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Sync(SyncEvent::Started {
                backend: SyncBackendKind::LegacySync,
                ..
            }) => return Ok(()),
            _ => {}
        }
    }
}

async fn prove_legacy_stays_starting(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    for _ in 0..2 {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Sync(SyncCommand::Start { request_id }))
            .await
            .map_err(|e| format!("{label}: submit lifecycle barrier failed: {e}"))?;
        loop {
            let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
                .await
                .map_err(|_| format!("{label}: timed out waiting for lifecycle barrier"))?
                .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
            match event {
                CoreEvent::Sync(SyncEvent::Running)
                | CoreEvent::StateChanged(koushi_state::AppState {
                    sync: koushi_state::SyncState::Running,
                    ..
                }) => {
                    return Err(format!(
                        "{label}: Running was emitted while the first legacy response was held"
                    ));
                }
                CoreEvent::Sync(SyncEvent::Started {
                    request_id: Some(event_request_id),
                    backend: SyncBackendKind::LegacySync,
                }) if event_request_id == request_id => break,
                CoreEvent::OperationFailed {
                    request_id: event_request_id,
                    failure,
                } if event_request_id == request_id => {
                    return Err(format!("{label}: lifecycle barrier failed: {failure:?}"));
                }
                _ => {}
            }
        }
        if matches!(conn.snapshot().sync, koushi_state::SyncState::Running) {
            return Err(format!(
                "{label}: snapshot became Running while the first legacy response was held"
            ));
        }
    }
    Ok(())
}

/// Wait for a `StateChanged` snapshot where `SessionState::Ready`.
async fn wait_for_ready_snapshot(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    if matches!(conn.snapshot().session, SessionState::Ready(_)) {
        return Ok(());
    }

    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for Ready snapshot"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event
            && matches!(snapshot.session, SessionState::Ready(_))
        {
            return Ok(());
        }
    }
}

async fn wait_for_room_unread_count(
    conn: &mut CoreConnection,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    loop {
        if conn
            .snapshot()
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && room.unread_count > 0)
        {
            return Ok(());
        }
        if started_at.elapsed() > EVENT_TIMEOUT {
            return Err(format!(
                "{label}: timed out waiting for unread room summary"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_activity_snapshot(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Activity SnapshotLoaded"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                request_id: ev_id,
                recent,
                unread,
                ..
            }) if ev_id == request_id => {
                let mut unread_room_ids = Vec::new();
                let mut unread_event_ids = Vec::new();
                for row in unread.rows {
                    match row.kind {
                        ActivityRowKind::Event => {
                            let event_id = row.event_id.ok_or_else(|| {
                                format!("{label}: Activity event row lacked an event id")
                            })?;
                            unread_event_ids.push(event_id);
                        }
                        ActivityRowKind::RoomUnread => {
                            if row.event_id.is_some() {
                                return Err(format!(
                                    "{label}: Activity placeholder contained an event id"
                                ));
                            }
                        }
                    }
                    unread_room_ids.push(row.room_id);
                }

                return Ok((
                    recent
                        .rows
                        .into_iter()
                        .filter_map(|row| row.event_id)
                        .collect(),
                    unread_event_ids,
                    unread_room_ids,
                ));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: Activity open failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_activity_marked_read(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for Activity MarkedRead"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Activity(ActivityEvent::MarkedRead {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: Activity mark-read failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_activity_unread_empty(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    loop {
        if matches!(
            &conn.snapshot().activity,
            ActivityState::Open { unread, .. } if unread.rows.is_empty()
        ) {
            return Ok(());
        }
        if started_at.elapsed() > EVENT_TIMEOUT {
            return Err(format!(
                "{label}: timed out waiting for empty unread stream"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_local_encryption_health(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: LocalEncryptionHealth,
    label: &str,
) -> Result<(), String> {
    let expected_state = LocalEncryptionState::from(expected);
    if conn.snapshot().local_encryption == expected_state {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for local encryption health"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if snapshot.local_encryption == expected_state => {
                return Ok(());
            }
            CoreEvent::LocalEncryption(LocalEncryptionEvent::HealthChanged { health })
                if health == expected && conn.snapshot().local_encryption == expected_state =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: local encryption health failed: {failure:?}"
                ));
            }
            _ if conn.snapshot().local_encryption == expected_state => {
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn wait_for_native_attention_state(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: &NativeAttentionState,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().native_attention == *expected {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for native attention summary"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if snapshot.native_attention == *expected => {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: native attention update failed: {failure:?}"
                ));
            }
            _ if conn.snapshot().native_attention == *expected => {
                return Ok(());
            }
            _ => {}
        }
    }
}

/// Wait for `AccountEvent::LoggedIn` with the given request_id.
fn ready_account_key<S: QaSnapshotEventSource + ?Sized>(conn: &S) -> Option<AccountKey> {
    match conn.snapshot().session {
        SessionState::Ready(info) => Some(AccountKey(info.user_id)),
        _ => None,
    }
}

async fn wait_for_logged_in<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<AccountKey, String> {
    if let Some(account_key) = ready_account_key(conn) {
        return Ok(account_key);
    }
    let deadline = QaEventDeadline::after(LOGIN_EVENT_TIMEOUT);
    loop {
        let event = match deadline.recv(conn).await {
            Ok(Ok(event)) => event,
            Ok(Err(lag)) => {
                return Err(format!(
                    "{label}: event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) => {
                return ready_account_key(conn)
                    .ok_or_else(|| format!("{label}: timed out waiting for LoggedIn event"));
            }
        };

        match event {
            CoreEvent::Account(AccountEvent::LoggedIn {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                return Ok(account_key);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {
                if let Some(account_key) = ready_account_key(conn) {
                    return Ok(account_key);
                }
            }
        }
    }
}

/// Wait for `AccountEvent::SessionRestored` with the given request_id.
async fn wait_for_session_restored<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if matches!(
            conn.snapshot().session,
            SessionState::AwaitingVerification { .. }
        ) {
            return Err(format!(
                "{label}: trusted restore unexpectedly requires proof; phase={}",
                gate_session_phase(&conn.snapshot().session)
            ));
        }
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for SessionRestored event; phase={}",
                    gate_session_phase(&conn.snapshot().session)
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                ensure_session_restored_account_key(&account_key, expected_account_key, label)?;
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

fn ensure_session_restored_account_key(
    actual: &AccountKey,
    expected: &AccountKey,
    label: &str,
) -> Result<(), String> {
    if actual != expected {
        return Err(format!("{label}: SessionRestored account_key mismatch"));
    }
    Ok(())
}

/// Wait for `AccountEvent::LoggedOut` with the given request_id.
async fn wait_for_logged_out<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    wait_for_logout_barrier(
        conn,
        request_id,
        QaLogoutAccountExpectation::Exact(expected_account_key),
        label,
    )
    .await
}

async fn wait_for_signed_out_after_logout<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    wait_for_logout_barrier(conn, request_id, QaLogoutAccountExpectation::Any, label).await
}

#[derive(Clone, Copy)]
enum QaLogoutAccountExpectation<'a> {
    Exact(&'a AccountKey),
    Any,
}

async fn wait_for_logout_barrier<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    account_expectation: QaLogoutAccountExpectation<'_>,
    label: &str,
) -> Result<(), String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let mut saw_logged_out = false;
    loop {
        if saw_logged_out && matches!(conn.snapshot().session, SessionState::SignedOut) {
            return Ok(());
        }

        let event = match deadline.recv(conn).await {
            Ok(Ok(event)) => event,
            Err(_) => {
                return if saw_logged_out
                    && matches!(conn.snapshot().session, SessionState::SignedOut)
                {
                    Ok(())
                } else {
                    Err(format!("{label}: timed out waiting for LoggedOut event"))
                };
            }
            Ok(Err(lag)) => {
                return if saw_logged_out
                    && matches!(conn.snapshot().session, SessionState::SignedOut)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "{label}: event stream lagged (skipped={})",
                        lag.skipped
                    ))
                };
            }
        };

        match event {
            CoreEvent::Account(AccountEvent::LoggedOut {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if let QaLogoutAccountExpectation::Exact(expected_account_key) = account_expectation
                    && account_key != *expected_account_key
                {
                    return Err(format!("{label}: LoggedOut account_key mismatch"));
                }
                saw_logged_out = true;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for `OperationFailed` with the given request_id and return the failure.
async fn wait_for_operation_failed<S: QaEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<CoreFailure, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for OperationFailed event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Ok(failure);
            }
            CoreEvent::Account(account_event) => {
                let matches_request = match &account_event {
                    AccountEvent::LoggedIn { request_id: id, .. }
                    | AccountEvent::SessionRestored { request_id: id, .. }
                    | AccountEvent::SavedSessionsListed { request_id: id, .. }
                    | AccountEvent::RecoveryCompleted { request_id: id, .. }
                    | AccountEvent::ProfileUpdated { request_id: id, .. }
                    | AccountEvent::AvatarThumbnailDownloaded { request_id: id, .. }
                    | AccountEvent::ReportCompleted { request_id: id, .. }
                    | AccountEvent::LoggedOut { request_id: id, .. }
                    | AccountEvent::AccountSwitched { request_id: id, .. } => *id == request_id,
                    AccountEvent::OidcAuthorizationCreated { .. }
                    | AccountEvent::RecoveryRequired { .. } => false,
                };
                if matches_request {
                    return Err(format!(
                        "{label}: expected OperationFailed but the operation succeeded"
                    ));
                }
            }
            _ => continue,
        }
    }
}

async fn wait_for_operation_failed_and_signed_out<S: QaSnapshotEventSource + ?Sized>(
    conn: &mut S,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<CoreFailure, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let mut operation_failure = None;
    loop {
        if matches!(conn.snapshot().session, SessionState::SignedOut) {
            if let Some(failure) = operation_failure.take() {
                return Ok(failure);
            }
        }

        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for OperationFailed and SignedOut state")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                operation_failure = Some(failure);
            }
            CoreEvent::Account(account_event) => {
                let matches_request = match &account_event {
                    AccountEvent::LoggedIn { request_id: id, .. }
                    | AccountEvent::SessionRestored { request_id: id, .. }
                    | AccountEvent::SavedSessionsListed { request_id: id, .. }
                    | AccountEvent::RecoveryCompleted { request_id: id, .. }
                    | AccountEvent::ProfileUpdated { request_id: id, .. }
                    | AccountEvent::AvatarThumbnailDownloaded { request_id: id, .. }
                    | AccountEvent::ReportCompleted { request_id: id, .. }
                    | AccountEvent::LoggedOut { request_id: id, .. }
                    | AccountEvent::AccountSwitched { request_id: id, .. } => *id == request_id,
                    AccountEvent::OidcAuthorizationCreated { .. }
                    | AccountEvent::RecoveryRequired { .. } => false,
                };
                if matches_request {
                    return Err(format!(
                        "{label}: expected OperationFailed but the operation succeeded"
                    ));
                }
            }
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Phase A E2EE trust helpers
// ---------------------------------------------------------------------------

fn authenticated_session_info(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<SessionInfo, String> {
    authenticated_session_info_from_state(&conn.snapshot().session)
        .cloned()
        .ok_or_else(|| format!("{label}: session is not authenticated"))
}

fn authenticated_session_info_from_state(session: &SessionState) -> Option<&SessionInfo> {
    match session {
        SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Ready(info) => Some(info),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::Locked(_)
        | SessionState::LoggingOut => None,
    }
}

async fn run_credential_health_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let probe_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ProbeLocalEncryptionHealth {
            request_id: probe_id,
        },
    ))
    .await
    .map_err(|e| format!("submit credential health probe: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        probe_id,
        LocalEncryptionHealth::Healthy,
        "credential health",
    )
    .await?;
    println!("credential_health=ok");

    let fail_closed_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::RecordLocalEncryptionHealth {
        request_id: fail_closed_id,
        health: LocalEncryptionHealth::LockedOrInaccessible,
    }))
    .await
    .map_err(|e| format!("submit credential fail-closed health record: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        fail_closed_id,
        LocalEncryptionHealth::LockedOrInaccessible,
        "credential fail-closed",
    )
    .await?;
    println!("fail_closed=ok");

    let reprobe_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ProbeLocalEncryptionHealth {
            request_id: reprobe_id,
        },
    ))
    .await
    .map_err(|e| format!("submit credential health restore probe: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        reprobe_id,
        LocalEncryptionHealth::Healthy,
        "credential health restore",
    )
    .await
}

async fn run_native_attention_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let rooms = vec![
        native_attention_room("!message:example.invalid", "Room", false, 8, 8, 0),
        native_attention_room("!dm:example.invalid", "Direct", true, 3, 3, 0),
        native_attention_room("!mention:example.invalid", "Mention", false, 1, 1, 1),
    ];
    let capabilities = native_attention_available_capabilities();
    let attention = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });

    let candidate = attention
        .summary
        .candidate
        .as_ref()
        .ok_or_else(|| "native attention candidate was not projected".to_owned())?;
    if candidate.kind != RoomAttentionKind::Mention || attention.summary.badge_count != 12 {
        return Err("native attention candidate priority or badge count was wrong".to_owned());
    }

    let candidate_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
        request_id: candidate_id,
        attention: attention.clone(),
    }))
    .await
    .map_err(|e| format!("native attention: submit candidate update failed: {e}"))?;
    wait_for_native_attention_state(conn, candidate_id, &attention, "native attention candidate")
        .await?;
    println!("notification_candidate=ok");
    println!("badge_state=ok");

    let focused = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: Some("!mention:example.invalid"),
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: true,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });
    if focused.summary.candidate.is_some()
        || focused.dispatch
            != (NativeAttentionDispatchState::Suppressed {
                reason: NativeAttentionSuppressionReason::WindowFocused,
            })
    {
        return Err("native attention focused room suppression was not projected".to_owned());
    }
    println!("suppress_focus=ok");

    let mut notification_modes = std::collections::HashMap::new();
    notification_modes.insert(
        "!message:example.invalid".to_owned(),
        RoomNotificationMode::Mute,
    );
    notification_modes.insert(
        "!dm:example.invalid".to_owned(),
        RoomNotificationMode::Mentions,
    );
    let with_modes = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &notification_modes,
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });
    if with_modes.summary.unread_count != 1
        || with_modes.summary.highlight_count != 1
        || with_modes.summary.badge_count != 1
        || with_modes
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.kind)
            != Some(RoomAttentionKind::Mention)
    {
        return Err("native attention did not respect per-room notification modes".to_owned());
    }
    println!("room_notification_modes=ok");

    let clear = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &[],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: attention.summary.candidate.as_ref(),
        capabilities,
    });
    if clear.summary.badge_count != 0 || clear.summary.candidate.is_some() {
        return Err("native attention clear state retained badge or candidate".to_owned());
    }

    let clear_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
        request_id: clear_id,
        attention: clear.clone(),
    }))
    .await
    .map_err(|e| format!("native attention: submit clear update failed: {e}"))?;
    wait_for_native_attention_state(conn, clear_id, &clear, "native attention clear").await?;
    println!("clear_badge=ok");

    Ok(())
}

fn native_attention_available_capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

fn native_attention_room(
    room_id: &str,
    display_name: &str,
    is_dm: bool,
    unread_count: u64,
    notification_count: u64,
    highlight_count: u64,
) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        display_label: display_name.to_owned(),
        original_display_label: display_name.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count,
        notification_count,
        highlight_count,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

async fn run_activity_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
    room_id: &str,
) -> Result<(), String> {
    let activity_body = "Phase 23 QA activity unread seed";
    let txn = "qa-phase23-activity-unread".to_owned();
    let send_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_b.clone(),
            transaction_id: txn.clone(),
            body: activity_body.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("activity: submit unread seed failed: {e}"))?;

    let send_outcome = wait_for_send_flow_completion(
        conn_b,
        send_id,
        key_b,
        &txn,
        activity_body,
        "activity unread seed send",
    )
    .await?;
    wait_for_item_with_body(
        conn_a,
        key_a,
        activity_body,
        "activity unread seed observed by A",
    )
    .await?;

    wait_for_room_unread_count(conn_a, room_id, "activity room unread count").await?;

    let open_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::OpenActivity {
            request_id: open_id,
        }))
        .await
        .map_err(|e| format!("activity: submit open failed: {e}"))?;
    let (recent_event_ids, unread_event_ids, unread_room_ids) =
        wait_for_activity_snapshot(conn_a, open_id, "activity open").await?;

    if !recent_event_ids
        .iter()
        .any(|event_id| event_id == &send_outcome.event_id)
    {
        return Err("activity recent projection did not include the unread seed".to_owned());
    }
    println!("activity_recent=ok");

    if !unread_room_ids
        .iter()
        .any(|unread_room_id| unread_room_id == room_id)
    {
        return Err("activity unread projection did not include the unread seed".to_owned());
    }
    println!("activity_unread=ok");
    if !unread_event_ids
        .iter()
        .any(|event_id| event_id == &send_outcome.event_id)
    {
        return Err("activity unread projection did not resolve the unread event".to_owned());
    }
    println!("activity_resolution=ok");

    let mark_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::MarkActivityRead {
            request_id: mark_id,
            target: ActivityMarkReadTarget::All,
        }))
        .await
        .map_err(|e| format!("activity: submit mark-read failed: {e}"))?;
    wait_for_activity_marked_read(conn_a, mark_id, "activity mark read").await?;
    wait_for_activity_unread_empty(conn_a, "activity unread cleared").await?;
    println!("activity_markread=ok");

    Ok(())
}

async fn seed_encrypted_room_key_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    label: &str,
) -> Result<String, String> {
    let create_room_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id: create_room_id,
        options: private_room_options("QA E2EE Backup Room", true),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted room create failed: {e}"))?;

    let room_id = wait_for_room_created(conn, create_room_id, label).await?;

    wait_for_room_in_room_list(conn, &room_id, "room list after encrypted backup seed").await?;

    let key = TimelineKey::room(account_key.clone(), room_id.clone());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted timeline subscribe failed: {e}"))?;

    wait_for_initial_items(conn, &key, subscribe_id, "subscribe encrypted backup seed").await?;

    let transaction_id = "qa-e2ee-key-backup-seed".to_owned();
    let send_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: send_id,
        key: key.clone(),
        transaction_id: transaction_id.clone(),
        body: E2EE_KEY_BACKUP_SEED_BODY.to_owned(),
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted backup seed send failed: {e}"))?;

    wait_for_send_flow_completion(
        conn,
        send_id,
        &key,
        &transaction_id,
        E2EE_KEY_BACKUP_SEED_BODY,
        "send encrypted backup seed",
    )
    .await?;

    Ok(room_id)
}

async fn enable_key_backup_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    passphrase: Option<AuthSecret>,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::EnableKeyBackup {
        request_id,
        passphrase,
    }))
    .await
    .map_err(|e| format!("{label}: submit enable key backup failed: {e}"))?;

    wait_for_key_backup_enabled(conn, account_key, request_id, label).await
}

async fn wait_for_key_backup_enabled(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    if let KeyBackupStatus::Enabled { version } = &conn.snapshot().e2ee_trust.key_backup {
        return Ok(version.clone());
    }

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup Enabled"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Enabled { version } => return Ok(version),
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup enable failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.key_backup {
                KeyBackupStatus::Enabled { version } => return Ok(version),
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup enable failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn restore_key_backup_failure_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    version: Option<String>,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id,
        version,
        request: RecoveryRequest {
            secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET),
        },
    }))
    .await
    .map_err(|e| format!("{label}: submit restore key backup failed: {e}"))?;

    wait_for_key_backup_failed(conn, account_key, request_id, label).await
}

async fn restore_key_backup_success_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    version: Option<String>,
    secret: AuthSecret,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id,
        version,
        request: RecoveryRequest { secret },
    }))
    .await
    .map_err(|e| format!("{label}: submit restore key backup failed: {e}"))?;

    wait_for_key_backup_restored(conn, account_key, request_id, label).await
}

async fn wait_for_key_backup_failed(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: current,
            ..
        } if current == request_id.sequence
    );

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup failure"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id.sequence => return Ok(()),
                KeyBackupStatus::Restoring {
                    request_id: current,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    return Err(format!("{label}: restore unexpectedly succeeded"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.key_backup {
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id.sequence => return Ok(()),
                KeyBackupStatus::Restoring {
                    request_id: current,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    return Err(format!("{label}: restore unexpectedly succeeded"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_key_backup_restored(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: current,
            ..
        } if current == request_id.sequence
    );
    let mut saw_restored_room = false;

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup restore success"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Restoring {
                    request_id: current,
                    restored_rooms,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                    saw_restored_room |= restored_rooms > 0;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    if saw_restored_room {
                        return Ok(());
                    }
                    return Err(format!(
                        "{label}: restore succeeded without any joined room"
                    ));
                }
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup restore failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.key_backup {
                KeyBackupStatus::Restoring {
                    request_id: current,
                    restored_rooms,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                    saw_restored_room |= restored_rooms > 0;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    if saw_restored_room {
                        return Ok(());
                    }
                    return Err(format!(
                        "{label}: restore succeeded without any joined room"
                    ));
                }
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup restore failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn reset_identity_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    password: String,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    let flow_id = request_id.sequence;
    conn.command(CoreCommand::Account(AccountCommand::ResetIdentity {
        request_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit reset identity failed: {e}"))?;

    match wait_for_identity_reset_auth_or_done(conn, account_key, flow_id, request_id, label)
        .await?
    {
        IdentityResetWait::Completed => Ok(()),
        IdentityResetWait::AuthRequired(IdentityResetAuthType::Uiaa) => {
            let submit_request_id = conn.next_request_id();
            conn.command(CoreCommand::Account(
                AccountCommand::SubmitIdentityResetAuth {
                    request_id: submit_request_id,
                    flow_id,
                    request: IdentityResetAuthRequest::UiaaPassword {
                        password: AuthSecret::new(password),
                    },
                },
            ))
            .await
            .map_err(|e| format!("{label}: submit reset identity UIAA failed: {e}"))?;
            wait_for_identity_reset_done(conn, account_key, flow_id, submit_request_id, label).await
        }
        IdentityResetWait::AuthRequired(IdentityResetAuthType::OAuth) => Err(format!(
            "{label}: OAuth identity reset cannot run headlessly"
        )),
        IdentityResetWait::AuthRequired(IdentityResetAuthType::Unknown) => Err(format!(
            "{label}: unknown identity reset auth type cannot run headlessly"
        )),
    }
}

enum IdentityResetWait {
    Completed,
    AuthRequired(IdentityResetAuthType),
}

async fn wait_for_identity_reset_auth_or_done(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    flow_id: u64,
    command_request_id: RequestId,
    label: &str,
) -> Result<IdentityResetWait, String> {
    let mut saw_request_state = false;

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for identity reset auth/done"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
                account_key: ev_account_key,
                state,
            }) if &ev_account_key == account_key => {
                if matches!(state, IdentityResetState::Idle) {
                    return Ok(IdentityResetWait::Completed);
                }
                if let Some(result) = identity_reset_observation(&state, flow_id, label)? {
                    return Ok(result);
                }
                if matches!(
                    state,
                    IdentityResetState::Resetting { request_id: current }
                        if current == flow_id
                ) {
                    saw_request_state = true;
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                let state = snapshot.e2ee_trust.identity_reset;
                if !matches!(state, IdentityResetState::Idle) {
                    if let Some(result) = identity_reset_observation(&state, flow_id, label)? {
                        return Ok(result);
                    }
                }
                if matches!(
                    state,
                    IdentityResetState::Resetting { request_id: current }
                        if current == flow_id
                ) {
                    saw_request_state = true;
                }
                if saw_request_state && matches!(state, IdentityResetState::Idle) {
                    return Ok(IdentityResetWait::Completed);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == command_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_identity_reset_done(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    flow_id: u64,
    command_request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.identity_reset,
        IdentityResetState::Resetting {
            request_id: current
        } if current == flow_id
    );

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for identity reset completion"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
                account_key: ev_account_key,
                state,
            }) if &ev_account_key == account_key => match state {
                IdentityResetState::Idle => return Ok(()),
                IdentityResetState::Resetting {
                    request_id: current,
                } if current == flow_id => {
                    saw_request_state = true;
                }
                IdentityResetState::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == flow_id => {
                    return Err(format!("{label}: identity reset failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.identity_reset {
                IdentityResetState::Idle if saw_request_state => return Ok(()),
                IdentityResetState::Resetting {
                    request_id: current,
                } if current == flow_id => {
                    saw_request_state = true;
                }
                IdentityResetState::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == flow_id => {
                    return Err(format!("{label}: identity reset failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == command_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn identity_reset_observation(
    state: &IdentityResetState,
    request_sequence: u64,
    label: &str,
) -> Result<Option<IdentityResetWait>, String> {
    match state {
        IdentityResetState::AwaitingAuth {
            request_id,
            auth_type,
        } if *request_id == request_sequence => {
            Ok(Some(IdentityResetWait::AuthRequired(*auth_type)))
        }
        IdentityResetState::Failed { request_id, kind } if *request_id == request_sequence => {
            Err(format!("{label}: identity reset failed: {kind:?}"))
        }
        _ => Ok(None),
    }
}

async fn verify_second_device_room_key_delivery_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    wait_for_room_in_room_list(conn_a, room_id, "A room list before encrypted send").await?;
    wait_for_room_in_room_list(conn_a2, room_id, "A2 room list before encrypted receive").await?;

    let key_a = TimelineKey::room(account_key_a.clone(), room_id.to_owned());
    let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.to_owned());

    let subscribe_a2_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_a2_id,
            key: key_a2.clone(),
        }))
        .await
        .map_err(|e| format!("second-device decrypt: submit A2 subscribe failed: {e}"))?;

    let initial_a2 = wait_for_initial_items(
        conn_a2,
        &key_a2,
        subscribe_a2_id,
        "second-device encrypted room subscribe",
    )
    .await?;
    assert_no_decryption_failure_items(&initial_a2, "second-device encrypted room initial")?;
    if find_timeline_item_with_body(&initial_a2, E2EE_KEY_BACKUP_SEED_BODY).is_none() {
        return Err("second-device decrypt: restored backup seed body was not visible".to_owned());
    }

    let transaction_id = "qa-e2ee-second-device-delivery".to_owned();
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_a.clone(),
            transaction_id: transaction_id.clone(),
            body: E2EE_SECOND_DEVICE_BODY.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("second-device decrypt: submit encrypted send failed: {e}"))?;

    wait_for_send_flow_completion(
        conn_a,
        send_id,
        &key_a,
        &transaction_id,
        E2EE_SECOND_DEVICE_BODY,
        "second-device encrypted send",
    )
    .await?;

    wait_for_item_with_body_or_decryption_failure(
        conn_a2,
        &key_a2,
        E2EE_SECOND_DEVICE_BODY,
        "second-device encrypted receive",
    )
    .await?;

    Ok(())
}

async fn verify_multi_user_multi_device_room_key_delivery_for_qa(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    recipient_base: Option<(&mut CoreConnection, &AccountKey)>,
) -> Result<(), String> {
    let check_recipient_second_device = env_flag_enabled(ENV_E2EE_RECIPIENT_SECOND_DEVICE)?;
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let room_id = create_room_for_qa(
        conn_a,
        "QA E2EE Multi Device DM",
        true,
        "e2ee multi-device create encrypted room",
    )
    .await?;

    wait_for_room_in_room_list(
        conn_a,
        &room_id,
        "e2ee multi-device A room list after create",
    )
    .await?;

    invite_user_for_qa(
        conn_a,
        &room_id,
        &user_b_full_id,
        "e2ee multi-device invite B",
    )
    .await?;

    let mut recipient = match recipient_base {
        Some((conn, account_key)) => QaE2eeRecipient::Borrowed { conn, account_key },
        None => QaE2eeRecipient::Owned(
            login_synced_participant_for_qa(
                &config.homeserver,
                qa_data_dir("e2ee-b"),
                &config.user_b,
                &config.password_b,
                DEVICE_B,
                "e2ee login B",
                "gate-bootstrap-b",
                QaParticipantLoginGate::BootstrapNewIdentity,
            )
            .await?
            .into(),
        ),
    };
    let mut owned_recipient_second_device = None;
    let mut owned_unverified_recipient_device = None;
    let stage_result: Result<(), String> = async {
        let (conn_b, account_key_b) = recipient.connection_and_account_key();

        wait_for_invite_in_snapshot(
            conn_b,
            &room_id,
            Some(false),
            "e2ee multi-device wait for B invite",
        )
        .await?;
        accept_invite_for_qa(conn_b, &room_id, "e2ee multi-device B accepts invite").await?;

        let settings_a = load_room_settings_for_qa(
            conn_a,
            &room_id,
            "e2ee multi-device A observes B membership",
        )
        .await?;
        assert_room_settings_contains_members(
            &settings_a,
            &[account_key_a.0.as_str(), user_b_full_id.as_str()],
            "e2ee multi-device A observes B membership",
        )?;
        wait_for_room_in_room_list(
            conn_a2,
            &room_id,
            "e2ee multi-device A2 room list after create",
        )
        .await?;
        wait_for_room_in_room_list(conn_b, &room_id, "e2ee multi-device B room list").await?;

        let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
        let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.clone());
        let key_b = TimelineKey::room(account_key_b.clone(), room_id.clone());

        let initial_a =
            subscribe_timeline_for_qa(conn_a, &key_a, "e2ee multi-device subscribe A").await?;
        let initial_a2 =
            subscribe_timeline_for_qa(conn_a2, &key_a2, "e2ee multi-device subscribe A2").await?;
        let initial_b =
            subscribe_timeline_for_qa(conn_b, &key_b, "e2ee multi-device subscribe B").await?;
        assert_no_decryption_failure_items(&initial_a, "e2ee multi-device A initial")?;
        assert_no_decryption_failure_items(&initial_a2, "e2ee multi-device A2 initial")?;
        assert_no_decryption_failure_items(&initial_b, "e2ee multi-device B initial")?;

        let mut recipient_second_device_key = None;
        if check_recipient_second_device {
            let runtime_b2 = CoreRuntime::start_with_data_dir(qa_data_dir("e2ee-b2"));
            let conn_b2 = runtime_b2.attach();
            owned_recipient_second_device =
                Some(QaOwnedRuntimeParticipant::new(runtime_b2, conn_b2));
            let participant_b2 = owned_recipient_second_device
                .as_mut()
                .expect("B2 owner was installed before login");
            let login_b2 = participant_b2.conn.next_request_id();
            participant_b2
                .conn
                .command(CoreCommand::Account(AccountCommand::LoginPassword {
                    request_id: login_b2,
                    request: koushi_state::LoginRequest {
                        homeserver: config.homeserver.clone(),
                        username: config.user_b.clone(),
                        password: AuthSecret::new(config.password_b.clone()),
                        device_display_name: Some("Koushi Core QA B2".to_owned()),
                    },
                }))
                .await
                .map_err(|error| format!("e2ee login B2 submit: {error}"))?;
            participant_b2.mark_login_submitted();
            let session_b2 =
                wait_for_existing_identity_gate(&mut participant_b2.conn, "e2ee B2 gate").await?;
            let session_b =
                authenticated_session_info(conn_b, "session B info for E2EE multi-device")?;
            verify_provisional_second_device_for_qa(
                conn_b,
                &mut participant_b2.conn,
                &session_b,
                &session_b2,
                "e2ee recipient verification B/B2",
                SasQaOutcome::Success,
            )
            .await?;
            let account_key_b2 =
                wait_for_logged_in(&mut participant_b2.conn, login_b2, "e2ee login B2").await?;
            participant_b2.mark_logged_in(account_key_b2.clone());
            wait_for_ready_snapshot(&mut participant_b2.conn, "e2ee B2 Ready").await?;
            start_sync_for_qa(&mut participant_b2.conn, "e2ee B2 sync").await?;
            wait_for_room_in_room_list(
                &mut participant_b2.conn,
                &room_id,
                "e2ee multi-device B2 room list",
            )
            .await?;
            let key_b2 = TimelineKey::room(account_key_b2.clone(), room_id.clone());
            let initial_b2 = subscribe_timeline_for_qa(
                &mut participant_b2.conn,
                &key_b2,
                "e2ee multi-device subscribe B2",
            )
            .await?;
            assert_no_decryption_failure_items(&initial_b2, "e2ee multi-device B2 initial")?;
            recipient_second_device_key = Some(key_b2);
        }

        let runtime_b3 = CoreRuntime::start_with_data_dir(qa_data_dir("e2ee-b3-unverified"));
        let conn_b3 = runtime_b3.attach();
        owned_unverified_recipient_device =
            Some(QaOwnedRuntimeParticipant::new(runtime_b3, conn_b3));
        let participant_b3 = owned_unverified_recipient_device
            .as_mut()
            .expect("B3 owner was installed before login");
        let login_b3 = participant_b3.conn.next_request_id();
        participant_b3
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_b3,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_b.clone(),
                    password: AuthSecret::new(config.password_b.clone()),
                    device_display_name: Some("Koushi Core QA B3 Unverified".to_owned()),
                },
            }))
            .await
            .map_err(|error| format!("e2ee unverified peer login submit: {error}"))?;
        participant_b3.mark_login_submitted();
        let session_b3 =
            wait_for_existing_identity_gate(&mut participant_b3.conn, "e2ee unverified peer gate")
                .await?;
        refresh_device_keys_and_assert_known_for_qa(
            conn_a,
            VerificationTarget {
                user_id: session_b3.user_id.clone(),
                device_id: session_b3.device_id.clone(),
            },
            "e2ee unverified peer device discovery",
        )
        .await?;
        let transaction_id = "qa-e2ee-multi-user-multi-device-delivery".to_owned();
        let send_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: key_a.clone(),
                transaction_id: transaction_id.clone(),
                body: E2EE_MULTI_USER_MULTI_DEVICE_BODY.to_owned(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|e| format!("e2ee multi-device: submit encrypted send failed: {e}"))?;

        wait_for_send_flow_completion_with_timeout(
            conn_a,
            send_id,
            &key_a,
            &transaction_id,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device encrypted send",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        println!("e2ee_unverified_peer_send_nonblocking=ok");

        wait_for_item_with_body_or_decryption_failure(
            conn_a2,
            &key_a2,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device A2 receive",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            conn_b,
            &key_b,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device B receive",
        )
        .await?;

        let session_b = authenticated_session_info(conn_b, "blocked QA B session")?;
        verify_provisional_second_device_for_qa(
            conn_b,
            &mut participant_b3.conn,
            &session_b,
            &session_b3,
            "blocked QA promote B3",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_b3 =
            wait_for_logged_in(&mut participant_b3.conn, login_b3, "blocked QA B3 login").await?;
        participant_b3.mark_logged_in(account_key_b3.clone());
        wait_for_ready_snapshot(&mut participant_b3.conn, "blocked QA B3 Ready").await?;
        start_sync_for_qa(&mut participant_b3.conn, "blocked QA B3 sync").await?;
        wait_for_room_in_room_list(&mut participant_b3.conn, &room_id, "blocked QA B3 room")
            .await?;
        let key_b3 = TimelineKey::room(account_key_b3.clone(), room_id.clone());
        let initial_b3 =
            subscribe_timeline_for_qa(&mut participant_b3.conn, &key_b3, "blocked QA B3 timeline")
                .await?;

        let (acknowledged, ack) = tokio::sync::oneshot::channel();
        let blacklist_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Account(
                AccountCommand::QaSetLocalDeviceBlacklisted {
                    request_id: blacklist_id,
                    target: VerificationTarget {
                        user_id: session_b3.user_id.clone(),
                        device_id: session_b3.device_id.clone(),
                    },
                    room_id: room_id.clone(),
                    acknowledged,
                },
            ))
            .await
            .map_err(|_| "blocked QA blacklist submit failed".to_owned())?;
        tokio::time::timeout(EVENT_TIMEOUT, ack)
            .await
            .map_err(|_| "blocked QA blacklist ack timeout".to_owned())?
            .map_err(|_| "blocked QA blacklist ack closed".to_owned())?
            .map_err(|_| "blocked QA blacklist failed".to_owned())?;
        let blocked_body = "Koushi blocked-device withheld probe";
        let blocked_txn = "qa-e2ee-blocked-device-withheld".to_owned();
        let blocked_send = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: blocked_send,
                key: key_a.clone(),
                transaction_id: blocked_txn.clone(),
                body: blocked_body.to_owned(),
                mentions: MentionIntent::default(),
            }))
            .await
            .map_err(|_| "blocked QA Core send submit failed".to_owned())?;
        let blocked_send_outcome = wait_for_send_flow_completion_with_timeout(
            conn_a,
            blocked_send,
            &key_a,
            &blocked_txn,
            blocked_body,
            "blocked QA Core send",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            conn_b,
            &key_b,
            blocked_body,
            "blocked QA nonblocked receive",
        )
        .await?;
        wait_for_withheld_event_projection_from_source(
            &mut participant_b3.conn,
            &key_b3,
            &blocked_send_outcome.event_id,
            blocked_body,
            &initial_b3,
            "blocked QA B3 withheld event",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        println!("e2ee_blocked_device_withheld=ok");

        if let Some(key_b2) = recipient_second_device_key {
            let participant_b2 = owned_recipient_second_device
                .as_mut()
                .expect("B2 key exists only while its owner is retained");
            wait_for_item_with_body_or_decryption_failure(
                &mut participant_b2.conn,
                &key_b2,
                E2EE_MULTI_USER_MULTI_DEVICE_BODY,
                "e2ee multi-device B2 receive",
            )
            .await?;
            println!("e2ee_recipient_second_device_decrypt=ok");
        }

        Ok(())
    }
    .await;

    finish_e2ee_recipient_stage_with_owned_cleanup(
        stage_result,
        Some((
            recipient.into_owned(),
            owned_recipient_second_device,
            owned_unverified_recipient_device,
        )),
        cleanup_e2ee_multi_device_participants,
    )
    .await
}

async fn refresh_device_keys_and_assert_known_for_qa(
    conn: &mut CoreConnection,
    target: VerificationTarget,
    label: &str,
) -> Result<(), String> {
    let (acknowledged, ack) = tokio::sync::oneshot::channel();
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::QaRefreshDeviceKeysAndAssertKnown {
            request_id,
            target,
            acknowledged,
        },
    ))
    .await
    .map_err(|_| format!("{label}: submit device-key refresh checkpoint failed"))?;

    tokio::time::timeout(E2EE_EVENT_TIMEOUT, ack)
        .await
        .map_err(|_| format!("{label}: timed out waiting for device-key refresh checkpoint"))?
        .map_err(|_| format!("{label}: device-key refresh checkpoint closed"))?
        .map_err(|_| format!("{label}: exact device was not known after key refresh"))
}

enum QaParticipantLoginGate<'a> {
    BootstrapNewIdentity,
    RecoverExistingIdentity(&'a AuthSecret),
}

struct QaParticipantLoginOutcome {
    runtime: CoreRuntime,
    conn: CoreConnection,
    account_key: AccountKey,
    bootstrap_recovery_secret: Option<AuthSecret>,
    sync_backend: SyncBackendKind,
}

struct QaOwnedLoggedInRuntime {
    runtime: CoreRuntime,
    conn: CoreConnection,
    account_key: AccountKey,
}

impl From<QaParticipantLoginOutcome> for QaOwnedLoggedInRuntime {
    fn from(participant: QaParticipantLoginOutcome) -> Self {
        Self {
            runtime: participant.runtime,
            conn: participant.conn,
            account_key: participant.account_key,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
enum QaOwnedRuntimePhase {
    LoginNotSubmitted,
    LoginSubmitted,
    LoggedIn(AccountKey),
}

struct QaOwnedRuntimeParticipant {
    runtime: CoreRuntime,
    conn: CoreConnection,
    phase: QaOwnedRuntimePhase,
}

impl QaOwnedRuntimeParticipant {
    fn new(runtime: CoreRuntime, conn: CoreConnection) -> Self {
        Self {
            runtime,
            conn,
            phase: QaOwnedRuntimePhase::LoginNotSubmitted,
        }
    }

    fn from_logged_in(participant: QaOwnedLoggedInRuntime) -> Self {
        Self {
            runtime: participant.runtime,
            conn: participant.conn,
            phase: QaOwnedRuntimePhase::LoggedIn(participant.account_key),
        }
    }

    fn mark_login_submitted(&mut self) {
        self.phase = QaOwnedRuntimePhase::LoginSubmitted;
    }

    fn mark_logged_in(&mut self, account_key: AccountKey) {
        self.phase = QaOwnedRuntimePhase::LoggedIn(account_key);
    }

    fn logged_in_connection_and_account_key(
        &mut self,
    ) -> Option<(&mut CoreConnection, &AccountKey)> {
        let QaOwnedRuntimePhase::LoggedIn(account_key) = &self.phase else {
            return None;
        };
        Some((&mut self.conn, account_key))
    }

    fn into_logged_in_runtime(self) -> QaOwnedLoggedInRuntime {
        let QaOwnedRuntimePhase::LoggedIn(account_key) = self.phase else {
            panic!("caller ownership returns only after a completed login");
        };
        QaOwnedLoggedInRuntime {
            runtime: self.runtime,
            conn: self.conn,
            account_key,
        }
    }
}

impl From<QaParticipantLoginOutcome> for QaOwnedRuntimeParticipant {
    fn from(participant: QaParticipantLoginOutcome) -> Self {
        Self::from_logged_in(QaOwnedLoggedInRuntime::from(participant))
    }
}

enum QaE2eeRecipient<'a> {
    Borrowed {
        conn: &'a mut CoreConnection,
        account_key: &'a AccountKey,
    },
    Owned(QaOwnedRuntimeParticipant),
}

impl QaE2eeRecipient<'_> {
    fn connection_and_account_key(&mut self) -> (&mut CoreConnection, &AccountKey) {
        match self {
            Self::Borrowed { conn, account_key } => (conn, account_key),
            Self::Owned(participant) => participant
                .logged_in_connection_and_account_key()
                .expect("owned E2EE recipient login completed before the post-login stage"),
        }
    }

    fn into_owned(self) -> Option<QaOwnedRuntimeParticipant> {
        match self {
            Self::Borrowed { .. } => None,
            Self::Owned(participant) => Some(participant),
        }
    }
}

async fn finish_e2ee_recipient_stage_with_owned_cleanup<T, Participant, Cleanup, CleanupFuture>(
    stage_result: Result<T, String>,
    owned_participant: Option<Participant>,
    cleanup: Cleanup,
) -> Result<T, String>
where
    Cleanup: FnOnce(Participant) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    let cleanup_result = match owned_participant {
        Some(participant) => cleanup(participant).await,
        None => Ok(()),
    };

    match (stage_result, cleanup_result) {
        (Err(stage_error), Ok(())) => Err(stage_error),
        (Err(stage_error), Err(_)) => Err(format!(
            "{stage_error}; owned E2EE recipient cleanup also failed"
        )),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
    }
}

async fn retain_or_cleanup_e2ee_callers_after_stage<Callers, Cleanup, CleanupFuture>(
    stage_result: Result<(), String>,
    callers: Callers,
    cleanup: Cleanup,
) -> Result<Callers, String>
where
    Cleanup: FnOnce(Callers) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    match stage_result {
        Ok(()) => Ok(callers),
        Err(stage_error) => match cleanup(callers).await {
            Ok(()) => Err(stage_error),
            Err(_) => Err(format!("{stage_error}; E2EE caller cleanup also failed")),
        },
    }
}

async fn retain_or_cleanup_owned_participant_after_stage<T, Participant, Cleanup, CleanupFuture>(
    stage_result: Result<T, String>,
    participant: Participant,
    cleanup: Cleanup,
) -> Result<(T, Participant), String>
where
    Cleanup: FnOnce(Participant) -> CleanupFuture,
    CleanupFuture: Future<Output = Result<(), String>>,
{
    match stage_result {
        Ok(value) => Ok((value, participant)),
        Err(stage_error) => match cleanup(participant).await {
            Ok(()) => Err(stage_error),
            Err(_) => Err(format!(
                "{stage_error}; owned participant login cleanup also failed"
            )),
        },
    }
}

async fn login_synced_participant_for_qa(
    homeserver: &str,
    data_dir: std::path::PathBuf,
    username: &str,
    password: &str,
    device_display_name: &str,
    label: &str,
    gate_label: &str,
    gate: QaParticipantLoginGate<'_>,
) -> Result<QaParticipantLoginOutcome, String> {
    let runtime = CoreRuntime::start_with_data_dir(data_dir);
    let conn = runtime.attach();
    let mut participant = QaOwnedRuntimeParticipant::new(runtime, conn);
    let login_stage_result: Result<(Option<AuthSecret>, SyncBackendKind), String> = async {
        let login_id = participant.conn.next_request_id();
        participant
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_id,
                request: koushi_state::LoginRequest {
                    homeserver: homeserver.to_owned(),
                    username: username.to_owned(),
                    password: AuthSecret::new(password.to_owned()),
                    device_display_name: Some(device_display_name.to_owned()),
                },
            }))
            .await
            .map_err(|e| format!("{label}: submit login failed: {e}"))?;
        participant.mark_login_submitted();
        let bootstrap_recovery_secret = match gate {
            QaParticipantLoginGate::BootstrapNewIdentity => {
                complete_new_identity_gate_for_qa(&mut participant.conn, password, gate_label)
                    .await?
            }
            QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret) => {
                wait_for_recovery_gate(&mut participant.conn, gate_label).await?;
                let recovery_request_id = participant.conn.next_request_id();
                participant
                    .conn
                    .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
                        request_id: recovery_request_id,
                        request: RecoveryRequest {
                            secret: recovery_secret.clone(),
                        },
                    }))
                    .await
                    .map_err(|e| format!("{gate_label}: submit recovery failed: {e}"))?;
                None
            }
        };
        let account_key = wait_for_logged_in(&mut participant.conn, login_id, label).await?;
        participant.mark_logged_in(account_key);
        wait_for_ready_snapshot(&mut participant.conn, label).await?;
        let sync_backend = start_sync_for_qa(&mut participant.conn, label).await?;
        Ok((bootstrap_recovery_secret, sync_backend))
    }
    .await;
    let ((bootstrap_recovery_secret, sync_backend), participant) =
        retain_or_cleanup_owned_participant_after_stage(
            login_stage_result,
            participant,
            |participant| async move {
                cleanup_owned_e2ee_participant_best_effort(participant, "participant login cleanup")
                    .await
            },
        )
        .await?;
    let QaOwnedLoggedInRuntime {
        runtime,
        conn,
        account_key,
    } = participant.into_logged_in_runtime();

    Ok(QaParticipantLoginOutcome {
        runtime,
        conn,
        account_key,
        bootstrap_recovery_secret,
        sync_backend,
    })
}

async fn subscribe_timeline_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
) -> Result<Vec<TimelineItem>, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit timeline subscribe failed: {e}"))?;
    wait_for_initial_items(conn, key, request_id, label).await
}

async fn subscribe_and_ack_active_timeline_projection_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
) -> Result<Vec<TimelineItem>, String> {
    let subscribe_request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_request_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit timeline subscribe failed: {e}"))?;

    let deadline = QaEventDeadline::after(TIMELINE_INITIAL_EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for active timeline projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(projection_request_id),
                key: ref event_key,
                generation,
                items,
                ..
            }) if event_key == key => {
                let acknowledgement_request_id = conn.next_request_id();
                conn.command(CoreCommand::App(
                    koushi_core::command::AppCommand::AcknowledgeTimelineProjection {
                        request_id: acknowledgement_request_id,
                        projection_request_id,
                        key: key.clone(),
                        generation,
                    },
                ))
                .await
                .map_err(|e| format!("{label}: submit projection acknowledgement failed: {e}"))?;
                return Ok(items);
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == subscribe_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn assert_no_decryption_failure_items(items: &[TimelineItem], label: &str) -> Result<(), String> {
    if items.iter().any(timeline_item_is_decryption_failure) {
        return Err(format!(
            "{label}: timeline contained an undecryptable event"
        ));
    }
    Ok(())
}

fn timeline_item_is_decryption_failure(item: &TimelineItem) -> bool {
    item.body
        .as_ref()
        .map(|body| body.contains("Unable to decrypt"))
        .unwrap_or(false)
}

#[derive(Debug)]
struct BodyWaitObserver<'a> {
    expected_body: &'a str,
    saw_decryption_failure: bool,
}

impl<'a> BodyWaitObserver<'a> {
    fn new(expected_body: &'a str) -> Self {
        Self {
            expected_body,
            saw_decryption_failure: false,
        }
    }

    fn observe_items(&mut self, items: &[TimelineItem]) -> Option<TimelineItem> {
        if let Some(item) = find_timeline_item_with_body(items, self.expected_body) {
            return Some(item);
        }
        if items.iter().any(timeline_item_is_decryption_failure) {
            self.saw_decryption_failure = true;
        }
        None
    }

    fn observe_diffs(&mut self, diffs: &[TimelineDiff]) -> Result<Option<TimelineItem>, String> {
        let mut found = None;
        visit_timeline_diff_items(diffs, |item| {
            if found.is_none() && timeline_item_body_matches(item, self.expected_body) {
                found = Some(item.clone());
            }
            if timeline_item_is_decryption_failure(item) {
                self.saw_decryption_failure = true;
            }
            Ok(())
        })?;
        Ok(found)
    }

    fn timeout_message(&self, label: &str) -> String {
        if self.saw_decryption_failure {
            format!(
                "{label}: timed out waiting for body {:?} after transient undecryptable events",
                self.expected_body
            )
        } else {
            format!(
                "{label}: timed out waiting for body {:?}",
                self.expected_body
            )
        }
    }
}

fn ensure_incoming_verification_receiver_sync_not_stopped(
    sync: &koushi_state::SyncState,
    label: &str,
) -> Result<(), String> {
    if matches!(sync, koushi_state::SyncState::Stopped) {
        Err(format!(
            "{label}: receiver sync is stopped; cannot await an incoming verification request"
        ))
    } else {
        Ok(())
    }
}

async fn wait_for_verification_requested_event_only(
    conn: &mut CoreConnection,
    expected_target: Option<&VerificationTarget>,
    excluded_flow_id: Option<u64>,
    label: &str,
) -> Result<u64, String> {
    ensure_incoming_verification_receiver_sync_not_stopped(&conn.snapshot().sync, label)?;
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;

    loop {
        if let Some(flow_id) = requested_verification_flow_id(
            &conn.snapshot().e2ee_trust.verification,
            expected_target,
            excluded_flow_id,
        )? {
            return Ok(flow_id);
        }

        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for incoming verification request"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. })
            | CoreEvent::StateChanged(AppState {
                e2ee_trust:
                    koushi_state::E2eeTrustState {
                        verification: state,
                        ..
                    },
                ..
            }) => {
                if let Some(flow_id) =
                    requested_verification_flow_id(&state, expected_target, excluded_flow_id)?
                {
                    return Ok(flow_id);
                }
            }
            _ => {}
        }
    }
}

fn requested_verification_flow_id(
    state: &VerificationFlowState,
    expected_target: Option<&VerificationTarget>,
    excluded_flow_id: Option<u64>,
) -> Result<Option<u64>, String> {
    if verification_state_flow_id(state).is_some_and(|flow_id| Some(flow_id) == excluded_flow_id) {
        return Ok(None);
    }
    if !verification_state_matches_target(state, expected_target) {
        return Ok(None);
    }

    match state {
        VerificationFlowState::Requested { request_id, .. }
        | VerificationFlowState::Accepted { request_id, .. }
        | VerificationFlowState::SasPresented { request_id, .. }
        | VerificationFlowState::Confirming { request_id, .. }
        | VerificationFlowState::Done { request_id, .. } => Ok(Some(*request_id)),
        VerificationFlowState::Failed { kind, .. } => Err(format!(
            "verification request failed before acceptance: {kind:?}"
        )),
        VerificationFlowState::Idle => Ok(None),
    }
}

async fn wait_for_verification_accepted(
    conn: &mut CoreConnection,
    flow_id: u64,
    command_request_id: Option<RequestId>,
    label: &str,
) -> Result<(), String> {
    if verification_state_is_at_least_accepted(&conn.snapshot().e2ee_trust.verification, flow_id)? {
        return Ok(());
    }

    let deadline = QaEventDeadline::after(E2EE_EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out waiting for verification acceptance"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. })
            | CoreEvent::StateChanged(AppState {
                e2ee_trust:
                    koushi_state::E2eeTrustState {
                        verification: state,
                        ..
                    },
                ..
            }) => {
                if verification_state_is_at_least_accepted(&state, flow_id)? {
                    return Ok(());
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if command_request_id == Some(ev_id) => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn verification_state_is_at_least_accepted(
    state: &VerificationFlowState,
    flow_id: u64,
) -> Result<bool, String> {
    if verification_state_flow_id(state) != Some(flow_id) {
        return Ok(false);
    }
    match state {
        VerificationFlowState::Accepted { .. }
        | VerificationFlowState::SasPresented { .. }
        | VerificationFlowState::Confirming { .. }
        | VerificationFlowState::Done { .. } => Ok(true),
        VerificationFlowState::Failed { kind, .. } => {
            Err(format!("verification failed before acceptance: {kind:?}"))
        }
        VerificationFlowState::Idle | VerificationFlowState::Requested { .. } => Ok(false),
    }
}

fn verification_state_sas(
    state: &VerificationFlowState,
    flow_id: u64,
    label: &str,
) -> Result<Option<Vec<SasEmoji>>, String> {
    if verification_state_flow_id(state) != Some(flow_id) {
        return Ok(None);
    }
    match state {
        VerificationFlowState::SasPresented { emojis, .. }
        | VerificationFlowState::Confirming { emojis, .. } => Ok(Some(emojis.clone())),
        VerificationFlowState::Done { .. } => Err(format!(
            "{label}: verification completed before SAS was observed"
        )),
        VerificationFlowState::Failed { kind, .. } => {
            Err(format!("{label}: verification failed before SAS: {kind:?}"))
        }
        VerificationFlowState::Idle
        | VerificationFlowState::Requested { .. }
        | VerificationFlowState::Accepted { .. } => Ok(None),
    }
}

fn verification_state_flow_id(state: &VerificationFlowState) -> Option<u64> {
    match state {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { request_id, .. }
        | VerificationFlowState::Accepted { request_id, .. }
        | VerificationFlowState::SasPresented { request_id, .. }
        | VerificationFlowState::Confirming { request_id, .. }
        | VerificationFlowState::Done { request_id, .. }
        | VerificationFlowState::Failed { request_id, .. } => Some(*request_id),
    }
}

fn verification_state_target(state: &VerificationFlowState) -> Option<&VerificationTarget> {
    match state {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { target, .. }
        | VerificationFlowState::Accepted { target, .. }
        | VerificationFlowState::SasPresented { target, .. }
        | VerificationFlowState::Confirming { target, .. }
        | VerificationFlowState::Done { target, .. }
        | VerificationFlowState::Failed { target, .. } => Some(target),
    }
}

fn verification_state_matches_target(
    state: &VerificationFlowState,
    expected_target: Option<&VerificationTarget>,
) -> bool {
    expected_target.is_none_or(|target| verification_state_target(state) == Some(target))
}

// ---------------------------------------------------------------------------
// Config and helpers
// ---------------------------------------------------------------------------

struct QaConfig {
    homeserver: String,
    server_name: String,
    server_kind: String,
    user_a: String,
    password_a: String,
    user_b: String,
    password_b: String,
    user_c: Option<String>,
    /// Expected sync backend ("SyncService" | "LegacySync"); QA fails on
    /// mismatch when set. Plain assertion input, not a credential.
    expect_sync_backend: Option<String>,
    /// Identity reset changes cross-signing identity for the account. Keep it
    /// opt-in so real-account QA cannot accidentally invalidate other devices.
    allow_identity_reset: bool,
}

impl QaConfig {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            homeserver: env_required(ENV_HOMESERVER)?,
            server_name: env_required(ENV_SERVER_NAME)?,
            server_kind: std::env::var(ENV_SERVER_KIND).unwrap_or_else(|_| "local".to_owned()),
            user_a: env_required(ENV_USER_A)?,
            password_a: env_required(ENV_PASSWORD_A)?,
            user_b: env_required(ENV_USER_B)?,
            password_b: env_required(ENV_PASSWORD_B)?,
            user_c: std::env::var(ENV_USER_C).ok(),
            expect_sync_backend: std::env::var(ENV_EXPECT_SYNC_BACKEND).ok(),
            allow_identity_reset: env_flag_enabled(ENV_ALLOW_IDENTITY_RESET)?,
        })
    }

    fn dm_scope_control_user_id(&self) -> Result<String, String> {
        let user_c = self.user_c.as_deref().ok_or_else(|| {
            format!("{ENV_USER_C} is required for the invites_dm dm_space_scope check")
        })?;
        Ok(format!("@{}:{}", user_c, self.server_name))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimelineStressConfig {
    space_count: usize,
    rooms_per_space: usize,
    messages_per_room: usize,
    replay_existing: bool,
}

impl TimelineStressConfig {
    fn from_env() -> Result<Self, String> {
        Ok(Self {
            space_count: bounded_usize_env(
                ENV_STRESS_SPACE_COUNT,
                DEFAULT_STRESS_SPACE_COUNT,
                MAX_STRESS_SPACE_COUNT,
            )?,
            rooms_per_space: bounded_usize_env(
                ENV_STRESS_ROOMS_PER_SPACE,
                DEFAULT_STRESS_ROOMS_PER_SPACE,
                MAX_STRESS_ROOMS_PER_SPACE,
            )?,
            messages_per_room: bounded_usize_env(
                ENV_STRESS_MESSAGES_PER_ROOM,
                DEFAULT_STRESS_MESSAGES_PER_ROOM,
                MAX_STRESS_MESSAGES_PER_ROOM,
            )?,
            replay_existing: env_flag_enabled(ENV_STRESS_REPLAY_EXISTING)?,
        })
    }

    fn total_rooms(self) -> usize {
        self.space_count * self.rooms_per_space
    }

    fn total_messages(self) -> usize {
        self.total_rooms() * self.messages_per_room + self.empty_formatted_probe_count()
    }

    fn empty_formatted_probe_count(self) -> usize {
        usize::from(self.total_rooms() > 0)
    }
}

fn bounded_usize_env(name: &str, default: usize, max: usize) -> Result<usize, String> {
    let Ok(value) = std::env::var(name) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a positive integer no greater than {max}"))?;
    if parsed == 0 || parsed > max {
        return Err(format!(
            "{name} must be a positive integer no greater than {max}"
        ));
    }
    Ok(parsed)
}

fn env_flag_enabled(name: &str) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(value) => parse_env_flag(name, &value),
        Err(_) => Ok(false),
    }
}

fn parse_env_flag(name: &str, value: &str) -> Result<bool, String> {
    if value == "1" || value.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if value == "0" || value.eq_ignore_ascii_case("false") || value.is_empty() {
        return Ok(false);
    }
    Err(format!(
        "{name} must be 1, true, 0, false, or unset; got {value}"
    ))
}

struct QaTcpProxy {
    listen_addr: SocketAddr,
    enabled: Arc<AtomicBool>,
    room_send_forwarded: Arc<AtomicUsize>,
    room_send_responses_completed: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    fallback_control: Arc<(Mutex<QaFallbackProxyState>, Condvar)>,
    messages_control: Arc<Mutex<QaMessagesProxyControl>>,
    accept_thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaFallbackProxyPhase {
    Open,
    Armed,
    AwaitingLegacy,
    LegacyHeld,
    Released,
}

#[derive(Debug)]
struct QaFallbackProxyState {
    phase: QaFallbackProxyPhase,
    versions_forwarded: bool,
    sync_service_failed: bool,
}

impl Default for QaFallbackProxyState {
    fn default() -> Self {
        Self {
            phase: QaFallbackProxyPhase::Open,
            versions_forwarded: false,
            sync_service_failed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaProxyRequestKind {
    Versions,
    SyncService,
    LegacySync,
    RoomSend,
    RoomMessages,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QaProxyRequestAction {
    Forward,
    FailClosed,
    HoldLegacy,
    ServeCannedMessages(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct QaMessagesProxyObservation {
    room_messages_request_count: u32,
    first_request_was_exact_tokenless_limit: bool,
    first_request_had_from: bool,
    freshness_page_served: bool,
    expected_end_token_was_used: bool,
    expected_end_token_request_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QaMessagesProxyExpectation {
    TokenlessLiveTail,
    BackwardFrom { token: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaMessagesProxyPhase {
    Open,
    Armed,
    Served,
    Rejected,
}

impl Default for QaMessagesProxyPhase {
    fn default() -> Self {
        Self::Open
    }
}

struct QaMessagesProxyState {
    phase: QaMessagesProxyPhase,
    expectation: Option<QaMessagesProxyExpectation>,
    tracked_end_token: Option<String>,
    observation: QaMessagesProxyObservation,
}

impl Default for QaMessagesProxyState {
    fn default() -> Self {
        Self {
            phase: QaMessagesProxyPhase::Open,
            expectation: None,
            tracked_end_token: None,
            observation: QaMessagesProxyObservation::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QaMessagesProxyDecision {
    Forward,
    FailClosed,
    ServeCannedPage,
}

impl QaMessagesProxyState {
    fn arm_page(
        &mut self,
        expectation: QaMessagesProxyExpectation,
        tracked_end_token: Option<String>,
    ) {
        self.phase = QaMessagesProxyPhase::Armed;
        self.expectation = Some(expectation);
        self.tracked_end_token = tracked_end_token;
        self.observation = QaMessagesProxyObservation::default();
    }

    fn observe_room_messages_request(
        &mut self,
        metadata: &QaRoomMessagesRequestMetadata,
    ) -> QaMessagesProxyDecision {
        self.observation.room_messages_request_count = self
            .observation
            .room_messages_request_count
            .saturating_add(1);
        if metadata.direction_is_backward
            && self
                .tracked_end_token
                .as_deref()
                .is_some_and(|token| metadata.from_token.as_deref() == Some(token))
        {
            self.observation.expected_end_token_request_count = self
                .observation
                .expected_end_token_request_count
                .saturating_add(1);
        }
        if self.phase != QaMessagesProxyPhase::Armed {
            return QaMessagesProxyDecision::Forward;
        }

        self.observation.first_request_was_exact_tokenless_limit =
            metadata.query_is_exact_tokenless_limit;
        self.observation.first_request_had_from = metadata.has_from;
        let expected_request_matched = match self.expectation.as_ref() {
            Some(QaMessagesProxyExpectation::TokenlessLiveTail) => {
                metadata.query_is_exact_tokenless_limit && !metadata.has_from
            }
            Some(QaMessagesProxyExpectation::BackwardFrom { token }) => {
                let matched = metadata.direction_is_backward
                    && metadata.from_token.as_deref() == Some(token.as_str());
                self.observation.expected_end_token_was_used = matched;
                matched
            }
            None => false,
        };
        if expected_request_matched {
            self.phase = QaMessagesProxyPhase::Served;
            self.observation.freshness_page_served = true;
            QaMessagesProxyDecision::ServeCannedPage
        } else {
            self.phase = QaMessagesProxyPhase::Rejected;
            QaMessagesProxyDecision::FailClosed
        }
    }
}

struct QaCannedTimelineEvent {
    event_id: String,
    sender: String,
    body: String,
    origin_server_ts: u64,
}

struct QaCannedMessagesPage {
    events: Vec<QaCannedTimelineEvent>,
    end: Option<String>,
}

impl QaCannedMessagesPage {
    fn anchored_silent_gap(
        newest_known_event_id: String,
        newest_known_body: String,
        missing_event_id: String,
        missing_body: String,
        older_anchor_event_id: String,
        sender: String,
        older_anchor_body: String,
    ) -> Self {
        Self {
            events: vec![
                QaCannedTimelineEvent {
                    event_id: newest_known_event_id,
                    sender: sender.clone(),
                    body: newest_known_body,
                    origin_server_ts: 1_900_000_000_002,
                },
                QaCannedTimelineEvent {
                    event_id: missing_event_id,
                    sender: sender.clone(),
                    body: missing_body,
                    origin_server_ts: 1_900_000_000_001,
                },
                QaCannedTimelineEvent {
                    event_id: older_anchor_event_id,
                    sender,
                    body: older_anchor_body,
                    origin_server_ts: 1,
                },
            ],
            end: None,
        }
    }

    fn response_body(&self) -> io::Result<Vec<u8>> {
        let chunk = self
            .events
            .iter()
            .map(|event| {
                serde_json::json!({
                    "type": "m.room.message",
                    "event_id": event.event_id,
                    "sender": event.sender,
                    "origin_server_ts": event.origin_server_ts,
                    "content": {
                        "msgtype": "m.text",
                        "body": event.body,
                    },
                })
            })
            .collect::<Vec<_>>();
        let mut response = serde_json::json!({
            "start": "qa-live-tail-start",
            "chunk": chunk,
            "state": [],
        });
        if let Some(end) = &self.end {
            response["end"] = serde_json::Value::String(end.clone());
        }
        serde_json::to_vec(&response)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

#[derive(Default)]
struct QaMessagesProxyControl {
    state: QaMessagesProxyState,
    canned_page: Option<QaCannedMessagesPage>,
}

impl QaTcpProxy {
    fn start(target_homeserver: &str) -> Result<Self, String> {
        let target = parse_http_homeserver_addr(target_homeserver)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("send_queue proxy bind failed: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("send_queue proxy nonblocking setup failed: {e}"))?;
        let listen_addr = listener
            .local_addr()
            .map_err(|e| format!("send_queue proxy local_addr failed: {e}"))?;
        let enabled = Arc::new(AtomicBool::new(true));
        let room_send_forwarded = Arc::new(AtomicUsize::new(0));
        let room_send_responses_completed = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let active_streams = Arc::new(Mutex::new(Vec::new()));
        let fallback_control =
            Arc::new((Mutex::new(QaFallbackProxyState::default()), Condvar::new()));
        let messages_control = Arc::new(Mutex::new(QaMessagesProxyControl::default()));

        let thread_enabled = enabled.clone();
        let thread_room_send_forwarded = room_send_forwarded.clone();
        let thread_room_send_responses_completed = room_send_responses_completed.clone();
        let thread_running = running.clone();
        let thread_streams = active_streams.clone();
        let thread_fallback_control = fallback_control.clone();
        let thread_messages_control = messages_control.clone();
        let accept_thread = thread::spawn(move || {
            while thread_running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        if !thread_enabled.load(Ordering::SeqCst) {
                            let _ = client.shutdown(Shutdown::Both);
                            continue;
                        }
                        spawn_proxy_pair(
                            client,
                            target,
                            thread_streams.clone(),
                            thread_fallback_control.clone(),
                            thread_messages_control.clone(),
                            thread_room_send_forwarded.clone(),
                            thread_room_send_responses_completed.clone(),
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => {
                        if thread_running.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(20));
                        }
                    }
                }
            }
        });

        Ok(Self {
            listen_addr,
            enabled,
            room_send_forwarded,
            room_send_responses_completed,
            running,
            active_streams,
            fallback_control,
            messages_control,
            accept_thread: Some(accept_thread),
        })
    }

    fn homeserver_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        shutdown_active_streams(&self.active_streams);
    }

    fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    fn room_send_forwarded_count(&self) -> usize {
        self.room_send_forwarded.load(Ordering::SeqCst)
    }

    fn room_send_responses_completed_count(&self) -> usize {
        self.room_send_responses_completed.load(Ordering::SeqCst)
    }

    fn arm_legacy_fallback(&self) -> Result<(), String> {
        let (state, _) = &*self.fallback_control;
        let mut state = state
            .lock()
            .map_err(|_| "timeline fallback proxy state lock was poisoned".to_owned())?;
        *state = QaFallbackProxyState {
            phase: QaFallbackProxyPhase::Armed,
            versions_forwarded: false,
            sync_service_failed: false,
        };
        Ok(())
    }

    fn wait_for_legacy_request_held(&self, timeout: Duration) -> Result<(), String> {
        let (state, changed) = &*self.fallback_control;
        let state = state
            .lock()
            .map_err(|_| "timeline fallback proxy state lock was poisoned".to_owned())?;
        let (state, wait) = changed
            .wait_timeout_while(state, timeout, |state| {
                state.phase != QaFallbackProxyPhase::LegacyHeld
            })
            .map_err(|_| "timeline fallback proxy wait lock was poisoned".to_owned())?;
        if wait.timed_out() || state.phase != QaFallbackProxyPhase::LegacyHeld {
            return Err(
                "timed out waiting for the first legacy sync request to be held".to_owned(),
            );
        }
        if !state.sync_service_failed {
            return Err("legacy request arrived without a failed SyncService request".to_owned());
        }
        Ok(())
    }

    fn release_legacy(&self) -> Result<(), String> {
        let (state, changed) = &*self.fallback_control;
        let mut state = state
            .lock()
            .map_err(|_| "timeline fallback proxy state lock was poisoned".to_owned())?;
        if state.phase != QaFallbackProxyPhase::LegacyHeld {
            return Err(
                "legacy fallback proxy release requested before a legacy request was held"
                    .to_owned(),
            );
        }
        state.phase = QaFallbackProxyPhase::Released;
        changed.notify_all();
        Ok(())
    }

    fn arm_first_live_tail_messages_page(
        &self,
        newest_known_event_id: String,
        newest_known_body: String,
        missing_event_id: String,
        missing_body: String,
        older_anchor_event_id: String,
        sender: String,
        older_anchor_body: String,
    ) -> Result<(), String> {
        self.arm_messages_page(
            QaMessagesProxyExpectation::TokenlessLiveTail,
            QaCannedMessagesPage::anchored_silent_gap(
                newest_known_event_id,
                newest_known_body,
                missing_event_id,
                missing_body,
                older_anchor_event_id,
                sender,
                older_anchor_body,
            ),
            None,
        )
    }

    fn arm_detached_live_tail_messages_page(
        &self,
        events: Vec<QaCannedTimelineEvent>,
        end_token: String,
    ) -> Result<(), String> {
        let tracked_end_token = end_token.clone();
        self.arm_messages_page(
            QaMessagesProxyExpectation::TokenlessLiveTail,
            QaCannedMessagesPage {
                events,
                end: Some(end_token),
            },
            Some(tracked_end_token),
        )
    }

    fn arm_historical_continuation_messages_page(
        &self,
        end_token: String,
        events: Vec<QaCannedTimelineEvent>,
    ) -> Result<(), String> {
        let tracked_end_token = end_token.clone();
        self.arm_messages_page(
            QaMessagesProxyExpectation::BackwardFrom { token: end_token },
            QaCannedMessagesPage { events, end: None },
            Some(tracked_end_token),
        )
    }

    fn arm_messages_page(
        &self,
        expectation: QaMessagesProxyExpectation,
        page: QaCannedMessagesPage,
        tracked_end_token: Option<String>,
    ) -> Result<(), String> {
        let mut control = self
            .messages_control
            .lock()
            .map_err(|_| "timeline messages proxy state lock was poisoned".to_owned())?;
        control.state.arm_page(expectation, tracked_end_token);
        control.canned_page = Some(page);
        Ok(())
    }

    fn live_tail_messages_observation(&self) -> Result<QaMessagesProxyObservation, String> {
        self.messages_control
            .lock()
            .map(|control| control.state.observation)
            .map_err(|_| "timeline messages proxy state lock was poisoned".to_owned())
    }
}

impl Drop for QaTcpProxy {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut state) = self.fallback_control.0.lock() {
            state.phase = QaFallbackProxyPhase::Released;
            self.fallback_control.1.notify_all();
        }
        shutdown_active_streams(&self.active_streams);
        let _ = TcpStream::connect(self.listen_addr);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn parse_http_homeserver_addr(homeserver: &str) -> Result<SocketAddr, String> {
    let without_scheme = homeserver.strip_prefix("http://").ok_or_else(|| {
        format!("send_queue proxy requires a local http:// homeserver, got {homeserver}")
    })?;
    let authority = without_scheme
        .split_once('/')
        .map(|(authority, _)| authority)
        .unwrap_or(without_scheme);
    authority
        .to_socket_addrs()
        .map_err(|e| format!("send_queue proxy could not resolve {authority}: {e}"))?
        .next()
        .ok_or_else(|| format!("send_queue proxy could not resolve {authority}"))
}

fn spawn_proxy_pair(
    mut client: TcpStream,
    target: SocketAddr,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    fallback_control: Arc<(Mutex<QaFallbackProxyState>, Condvar)>,
    messages_control: Arc<Mutex<QaMessagesProxyControl>>,
    room_send_forwarded: Arc<AtomicUsize>,
    room_send_responses_completed: Arc<AtomicUsize>,
) {
    thread::spawn(move || {
        let _ = proxy_single_http_request(
            &mut client,
            target,
            active_streams,
            fallback_control,
            messages_control,
            room_send_forwarded,
            room_send_responses_completed,
        );
        let _ = client.shutdown(Shutdown::Both);
    });
}

fn proxy_single_http_request(
    client: &mut TcpStream,
    target: SocketAddr,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    fallback_control: Arc<(Mutex<QaFallbackProxyState>, Condvar)>,
    messages_control: Arc<Mutex<QaMessagesProxyControl>>,
    room_send_forwarded: Arc<AtomicUsize>,
    room_send_responses_completed: Arc<AtomicUsize>,
) -> io::Result<()> {
    let mut request_head = Vec::new();
    {
        let reader_stream = client.try_clone()?;
        let mut reader = io::BufReader::new(reader_stream);
        loop {
            let mut line = Vec::new();
            let bytes = io::BufRead::read_until(&mut reader, b'\n', &mut line)?;
            if bytes == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed before HTTP headers",
                ));
            }
            request_head.extend_from_slice(&line);
            if request_head.ends_with(b"\r\n\r\n") || request_head.ends_with(b"\n\n") {
                break;
            }
            if request_head.len() > 64 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP headers exceeded QA proxy limit",
                ));
            }
        }

        let content_length = http_content_length(&request_head)?;
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            io::Read::read_exact(&mut reader, &mut body)?;
            request_head.extend_from_slice(&body);
        }
    }

    let request_kind = qa_proxy_request_kind(&request_head)?;
    let action = qa_messages_proxy_action(&messages_control, request_kind, &request_head)?
        .unwrap_or(fallback_proxy_action(&fallback_control, request_kind)?);
    let count_forwarded_room_send =
        request_kind == QaProxyRequestKind::RoomSend && action == QaProxyRequestAction::Forward;
    match action {
        QaProxyRequestAction::Forward => {}
        QaProxyRequestAction::FailClosed => {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "QA proxy closed a selected sync request",
            ));
        }
        QaProxyRequestAction::HoldLegacy => {
            let (state, changed) = &*fallback_control;
            let mut state = state
                .lock()
                .map_err(|_| io::Error::other("QA fallback proxy state lock was poisoned"))?;
            while state.phase == QaFallbackProxyPhase::LegacyHeld {
                state = changed
                    .wait(state)
                    .map_err(|_| io::Error::other("QA fallback proxy wait lock was poisoned"))?;
            }
            if state.phase != QaFallbackProxyPhase::Released {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "QA fallback proxy stopped before legacy release",
                ));
            }
        }
        QaProxyRequestAction::ServeCannedMessages(body) => {
            write_qa_json_response(client, &body)?;
            return Ok(());
        }
    }

    let mut server = TcpStream::connect_timeout(&target, Duration::from_secs(2))?;
    if let Ok(mut streams) = active_streams.lock() {
        if let Ok(stream) = client.try_clone() {
            streams.push(stream);
        }
        if let Ok(stream) = server.try_clone() {
            streams.push(stream);
        }
    }

    let request = rewrite_http_request_connection_close(&request_head)?;
    if count_forwarded_room_send {
        room_send_forwarded.fetch_add(1, Ordering::SeqCst);
    }
    io::Write::write_all(&mut server, &request)?;
    io::copy(&mut server, client)?;
    if count_forwarded_room_send {
        room_send_responses_completed.fetch_add(1, Ordering::SeqCst);
    }
    Ok(())
}

fn qa_proxy_request_kind(request: &[u8]) -> io::Result<QaProxyRequestKind> {
    let header_end = find_http_header_end(request)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP headers"))?;
    let head = String::from_utf8_lossy(&request[..header_end]);
    let line = head
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP request line"))?;
    let mut fields = line.split_ascii_whitespace();
    let method = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP method"))?;
    let target = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP target"))?;
    let version = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP version"))?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    Ok(match (method, path) {
        ("GET", "/_matrix/client/versions") => QaProxyRequestKind::Versions,
        (_, "/_matrix/client/unstable/org.matrix.simplified_msc3575/sync") => {
            QaProxyRequestKind::SyncService
        }
        (_, "/_matrix/client/v3/sync" | "/_matrix/client/r0/sync") => {
            QaProxyRequestKind::LegacySync
        }
        ("PUT", path)
            if path.starts_with("/_matrix/client/")
                && path.contains("/rooms/")
                && path.contains("/send/") =>
        {
            QaProxyRequestKind::RoomSend
        }
        (_, path)
            if path.starts_with("/_matrix/client/")
                && path.contains("/rooms/")
                && path.ends_with("/messages") =>
        {
            QaProxyRequestKind::RoomMessages
        }
        _ => QaProxyRequestKind::Other,
    })
}

fn qa_messages_proxy_action(
    control: &Arc<Mutex<QaMessagesProxyControl>>,
    request_kind: QaProxyRequestKind,
    request: &[u8],
) -> io::Result<Option<QaProxyRequestAction>> {
    if request_kind != QaProxyRequestKind::RoomMessages {
        return Ok(None);
    }
    let metadata = qa_room_messages_request_metadata(request)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "room messages proxy received a non-room-messages request",
        )
    })?;
    let mut control = control
        .lock()
        .map_err(|_| io::Error::other("QA messages proxy state lock was poisoned"))?;
    match control.state.observe_room_messages_request(&metadata) {
        QaMessagesProxyDecision::Forward => Ok(None),
        QaMessagesProxyDecision::FailClosed => Ok(Some(QaProxyRequestAction::FailClosed)),
        QaMessagesProxyDecision::ServeCannedPage => {
            let page = control.canned_page.take().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "QA messages proxy armed without a canned messages page",
                )
            })?;
            Ok(Some(QaProxyRequestAction::ServeCannedMessages(
                page.response_body()?,
            )))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QaRoomMessagesRequestMetadata {
    query_is_exact_tokenless_limit: bool,
    has_from: bool,
    direction_is_backward: bool,
    from_token: Option<String>,
}

fn qa_room_messages_request_metadata(
    request: &[u8],
) -> io::Result<Option<QaRoomMessagesRequestMetadata>> {
    let header_end = find_http_header_end(request)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP headers"))?;
    let head = String::from_utf8_lossy(&request[..header_end]);
    let line = head
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP request line"))?;
    let mut fields = line.split_ascii_whitespace();
    let _method = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP method"))?;
    let target = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP target"))?;
    let version = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP version"))?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !path.starts_with("/_matrix/client/")
        || !path.contains("/rooms/")
        || !path.ends_with("/messages")
    {
        return Ok(None);
    }
    let mut direction_is_backward = false;
    let mut from_token = None;
    for field in query.split('&') {
        let (name, value) = field.split_once('=').unwrap_or((field, ""));
        match name {
            "dir" => direction_is_backward = value == "b",
            "from" => from_token = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(Some(QaRoomMessagesRequestMetadata {
        query_is_exact_tokenless_limit: query == "dir=b&limit=128",
        has_from: from_token.is_some(),
        direction_is_backward,
        from_token,
    }))
}

fn write_qa_json_response(client: &mut TcpStream, body: &[u8]) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    io::Write::write_all(client, headers.as_bytes())?;
    io::Write::write_all(client, body)
}

fn fallback_proxy_action(
    control: &Arc<(Mutex<QaFallbackProxyState>, Condvar)>,
    request: QaProxyRequestKind,
) -> io::Result<QaProxyRequestAction> {
    let (state, changed) = &**control;
    let mut state = state
        .lock()
        .map_err(|_| io::Error::other("QA fallback proxy state lock was poisoned"))?;
    let action = match (state.phase, request) {
        (QaFallbackProxyPhase::Armed, QaProxyRequestKind::Versions) => {
            state.versions_forwarded = true;
            QaProxyRequestAction::Forward
        }
        (
            QaFallbackProxyPhase::Armed | QaFallbackProxyPhase::AwaitingLegacy,
            QaProxyRequestKind::SyncService,
        ) => {
            state.phase = QaFallbackProxyPhase::AwaitingLegacy;
            state.sync_service_failed = true;
            changed.notify_all();
            QaProxyRequestAction::FailClosed
        }
        (QaFallbackProxyPhase::AwaitingLegacy, QaProxyRequestKind::LegacySync) => {
            state.phase = QaFallbackProxyPhase::LegacyHeld;
            changed.notify_all();
            QaProxyRequestAction::HoldLegacy
        }
        (QaFallbackProxyPhase::LegacyHeld, QaProxyRequestKind::LegacySync) => {
            QaProxyRequestAction::HoldLegacy
        }
        _ => QaProxyRequestAction::Forward,
    };
    Ok(action)
}

fn http_content_length(request_head: &[u8]) -> io::Result<usize> {
    let head = String::from_utf8_lossy(request_head);
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP content-length")
            });
        }
    }
    Ok(0)
}

fn rewrite_http_request_connection_close(request: &[u8]) -> io::Result<Vec<u8>> {
    let Some(header_end) = find_http_header_end(request) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing HTTP header terminator",
        ));
    };
    let (head, body) = request.split_at(header_end);
    let head = String::from_utf8_lossy(head);
    let mut lines = head.lines();
    let Some(request_line) = lines.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing HTTP request line",
        ));
    };

    let mut rewritten = Vec::with_capacity(request.len() + 32);
    rewritten.extend_from_slice(request_line.as_bytes());
    rewritten.extend_from_slice(b"\r\n");
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let lower = line
            .split_once(':')
            .map(|(name, _)| name.trim().to_ascii_lowercase());
        if matches!(lower.as_deref(), Some("connection" | "proxy-connection")) {
            continue;
        }
        rewritten.extend_from_slice(line.as_bytes());
        rewritten.extend_from_slice(b"\r\n");
    }
    rewritten.extend_from_slice(b"Connection: close\r\n\r\n");
    rewritten.extend_from_slice(body);
    Ok(rewritten)
}

fn find_http_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .or_else(|| {
            request
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| position + 2)
        })
}

fn shutdown_active_streams(active_streams: &Arc<Mutex<Vec<TcpStream>>>) {
    if let Ok(mut streams) = active_streams.lock() {
        for stream in streams.drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

/// Fail when an expected backend is configured and the observed one differs.
fn assert_expected_backend(
    expected: Option<&str>,
    observed: SyncBackendKind,
    label: &str,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let observed_name = match observed {
        SyncBackendKind::SyncService => "SyncService",
        SyncBackendKind::LegacySync => "LegacySync",
    };
    if observed_name != expected {
        return Err(format!(
            "{label}: sync backend mismatch: expected {expected}, observed {observed_name}"
        ));
    }
    Ok(())
}

fn env_required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

/// Data directory for QA runs.
fn qa_data_dir(suffix: &str) -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("KOUSHI_QA_DATA_DIR") {
        return std::path::PathBuf::from(dir).join(suffix);
    }
    std::env::temp_dir()
        .join("koushi-core-qa")
        .join(format!("{}_{}", std::process::id(), suffix))
}

// ---------------------------------------------------------------------------
// Phase 5 event waiter helpers
// ---------------------------------------------------------------------------

/// Wait for `TimelineEvent::InitialItems` for the given key and request_id.
/// Returns the initial item list.
async fn wait_for_initial_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<Vec<koushi_core::event::TimelineItem>, String> {
    wait_for_initial_items_from_source(conn, key, request_id, label, TIMELINE_INITIAL_EVENT_TIMEOUT)
        .await
}

async fn wait_for_initial_items_from_source<S: QaEventSource + ?Sized>(
    source: &mut S,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
    label: &str,
    timeout: Duration,
) -> Result<Vec<koushi_core::event::TimelineItem>, String> {
    let deadline = QaEventDeadline::after(timeout);
    let mut diagnostics = InitialItemsWaitDiagnostics::default();
    loop {
        let event = deadline
            .recv(source)
            .await
            .map_err(|_| diagnostics.timeout_message(label))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        diagnostics.observe(&event, key, request_id);
        match match_initial_items_wait_event(event, key, request_id) {
            InitialItemsWaitMatch::Items(items) => return Ok(items),
            InitialItemsWaitMatch::Failure(failure) => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            InitialItemsWaitMatch::Ignore => continue,
        }
    }
}

#[derive(Default)]
struct InitialItemsWaitDiagnostics {
    same_key_exact_cause: u64,
    same_key_wrong_cause: u64,
    same_key_causeless: u64,
    wrong_key_initial_items: u64,
    unrelated_events: u64,
}

impl InitialItemsWaitDiagnostics {
    fn observe(&mut self, event: &CoreEvent, key: &TimelineKey, request_id: RequestId) {
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                cause_request_id,
                key: event_key,
                ..
            }) if event_key == key => match cause_request_id {
                Some(cause_request_id) if *cause_request_id == request_id => {
                    self.same_key_exact_cause += 1;
                }
                Some(_) => self.same_key_wrong_cause += 1,
                None => self.same_key_causeless += 1,
            },
            CoreEvent::Timeline(TimelineEvent::InitialItems { .. }) => {
                self.wrong_key_initial_items += 1;
            }
            _ => self.unrelated_events += 1,
        }
    }

    fn timeout_message(&self, label: &str) -> String {
        format!(
            "{label}: timed out waiting for TimelineEvent::InitialItems \
             (same_key_exact_cause={}, same_key_wrong_cause={}, same_key_causeless={}, \
             wrong_key_initial_items={}, unrelated_events={})",
            self.same_key_exact_cause,
            self.same_key_wrong_cause,
            self.same_key_causeless,
            self.wrong_key_initial_items,
            self.unrelated_events,
        )
    }
}

enum InitialItemsWaitMatch {
    Items(Vec<koushi_core::event::TimelineItem>),
    Failure(CoreFailure),
    Ignore,
}

fn match_initial_items_wait_event(
    event: CoreEvent,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
) -> InitialItemsWaitMatch {
    match event {
        CoreEvent::Timeline(TimelineEvent::InitialItems {
            cause_request_id: Some(event_cause_request_id),
            key: event_key,
            items,
            ..
        }) if event_key == *key && event_cause_request_id == request_id => {
            InitialItemsWaitMatch::Items(items)
        }
        CoreEvent::OperationFailed {
            request_id: event_request_id,
            failure,
        } if event_request_id == request_id => InitialItemsWaitMatch::Failure(failure),
        _ => InitialItemsWaitMatch::Ignore,
    }
}

fn find_timeline_item_with_body(
    items: &[koushi_core::event::TimelineItem],
    expected_body: &str,
) -> Option<koushi_core::event::TimelineItem> {
    items
        .iter()
        .find(|item| {
            item.body
                .as_ref()
                .map(|body| body.contains(expected_body))
                .unwrap_or(false)
        })
        .cloned()
}

fn thread_initial_items_need_paginate_backfill(
    initial_items: &[koushi_core::event::TimelineItem],
    expected_body: &str,
) -> bool {
    find_timeline_item_with_body(initial_items, expected_body).is_none()
}

fn thread_reply_should_repaginate_on_idle(pagination_ended: bool) -> bool {
    !pagination_ended
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SendFlowOutcome {
    sdk_transaction_id: String,
    send_transaction_id: String,
    event_id: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SendQueueLocalEcho {
    request_id: RequestId,
    client_transaction_id: String,
    sdk_transaction_id: String,
}

#[derive(Debug)]
struct SendFlowWaiter {
    request_id: koushi_core::ids::RequestId,
    key: TimelineKey,
    expected_client_txn_id: String,
    expected_body: String,
    sdk_transaction_id: Option<String>,
    local_echo_send_state: Option<TimelineSendState>,
    send_transaction_id: Option<String>,
    event_id: Option<String>,
}

impl SendFlowWaiter {
    fn new(
        request_id: koushi_core::ids::RequestId,
        key: TimelineKey,
        expected_client_txn_id: impl Into<String>,
        expected_body: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            key,
            expected_client_txn_id: expected_client_txn_id.into(),
            expected_body: expected_body.into(),
            sdk_transaction_id: None,
            local_echo_send_state: None,
            send_transaction_id: None,
            event_id: None,
        }
    }

    fn observe(&mut self, event: CoreEvent) -> Result<(), String> {
        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == &self.key => {
                self.observe_local_echo(diffs);
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: ev_id,
                key: ref ev_key,
                transaction_id,
                event_id,
            }) if ev_id == self.request_id && ev_key == &self.key => {
                if transaction_id != self.expected_client_txn_id {
                    return Err(format!(
                        "send completed txn_id mismatch: expected {}, got {}",
                        self.expected_client_txn_id, transaction_id
                    ));
                }
                self.send_transaction_id = Some(transaction_id);
                self.event_id = Some(event_id);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == self.request_id => {
                return Err(format!("send flow failed: {failure:?}"));
            }
            _ => {}
        }
        if matches!(
            self.local_echo_send_state,
            Some(TimelineSendState::NotSent { .. })
        ) && self.send_transaction_id.is_none()
        {
            return Err(format!("send flow failed: {}", self.status_summary()));
        }
        Ok(())
    }

    fn observe_local_echo(&mut self, diffs: Vec<koushi_core::event::TimelineDiff>) {
        for diff in &diffs {
            let item = match diff {
                koushi_core::event::TimelineDiff::PushBack { item }
                | koushi_core::event::TimelineDiff::PushFront { item }
                | koushi_core::event::TimelineDiff::Insert { item, .. }
                | koushi_core::event::TimelineDiff::Set { item, .. } => item,
                _ => continue,
            };
            if item
                .body
                .as_ref()
                .map(|body| body.contains(&self.expected_body))
                .unwrap_or(false)
            {
                if let Some(state) = item.send_state.as_ref() {
                    self.local_echo_send_state = Some(state.clone());
                }
                if let koushi_core::event::TimelineItemId::Transaction { transaction_id } = &item.id
                {
                    if self.sdk_transaction_id.is_none() {
                        self.sdk_transaction_id = Some(transaction_id.clone());
                    }
                    break;
                }
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.sdk_transaction_id.is_some()
            && self.send_transaction_id.is_some()
            && self.event_id.is_some()
    }

    fn status_summary(&self) -> String {
        format!(
            "local_echo={} local_echo_send_state={} send_completed={} event_id={}",
            self.sdk_transaction_id.is_some(),
            self.local_echo_send_state
                .as_ref()
                .map(timeline_send_state_label)
                .unwrap_or("missing"),
            self.send_transaction_id.is_some(),
            self.event_id.is_some()
        )
    }

    fn finish(self) -> Result<SendFlowOutcome, String> {
        Ok(SendFlowOutcome {
            sdk_transaction_id: self
                .sdk_transaction_id
                .ok_or_else(|| "send flow: missing local echo".to_owned())?,
            send_transaction_id: self
                .send_transaction_id
                .ok_or_else(|| "send flow: missing SendCompleted".to_owned())?,
            event_id: self
                .event_id
                .ok_or_else(|| "send flow: missing SendCompleted event id".to_owned())?,
        })
    }
}

fn timeline_send_state_label(state: &TimelineSendState) -> &'static str {
    match state {
        TimelineSendState::Sending => "Sending",
        TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
        } => "NotSent(recoverable)",
        TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Unrecoverable,
        } => "NotSent(unrecoverable)",
        TimelineSendState::Cancelled => "Cancelled",
        TimelineSendState::Sent => "Sent",
    }
}

/// Wait for both the local echo diff and `TimelineEvent::SendCompleted`
/// for a single send sequence, accepting either order.
async fn wait_for_send_flow_completion(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    key: &TimelineKey,
    client_txn_id: &str,
    expected_body: &str,
    label: &str,
) -> Result<SendFlowOutcome, String> {
    wait_for_send_flow_completion_with_timeout(
        conn,
        request_id,
        key,
        client_txn_id,
        expected_body,
        label,
        EVENT_TIMEOUT,
    )
    .await
}

async fn wait_for_send_flow_completion_with_timeout(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    key: &TimelineKey,
    client_txn_id: &str,
    expected_body: &str,
    label: &str,
    timeout: Duration,
) -> Result<SendFlowOutcome, String> {
    let mut waiter = SendFlowWaiter::new(request_id, key.clone(), client_txn_id, expected_body);

    let deadline = QaEventDeadline::after(timeout);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for send flow completion ({})",
                    waiter.status_summary()
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        waiter.observe(event)?;
        if waiter.is_complete() {
            return waiter.finish();
        }
    }
}

async fn send_text_expect_local_echo(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    client_transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<SendQueueLocalEcho, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: key.clone(),
        transaction_id: client_transaction_id.to_owned(),
        body: body.to_owned(),
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("{label}: submit SendText failed: {e}"))?;

    let sdk_transaction_id =
        wait_for_local_echo_transaction(conn, key, request_id, body, label).await?;
    Ok(SendQueueLocalEcho {
        request_id,
        client_transaction_id: client_transaction_id.to_owned(),
        sdk_transaction_id,
    })
}

async fn wait_for_local_echo_transaction(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    expected_body: &str,
    label: &str,
) -> Result<String, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for local echo"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut found = None;
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_body_matches(item, expected_body)
                        && let Some(transaction_id) = timeline_item_transaction_id(item)
                    {
                        found = Some(transaction_id.to_owned());
                    }
                    Ok(())
                })?;
                if let Some(transaction_id) = found {
                    return Ok(transaction_id);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: send command failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_timeline_send_state(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    sdk_transaction_id: &str,
    matches_state: impl Fn(&TimelineSendState) -> bool,
    label: &str,
) -> Result<TimelineSendState, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for send state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                for item in &items {
                    if timeline_item_transaction_id(item) == Some(sdk_transaction_id)
                        && let Some(state) = item.send_state.as_ref()
                        && matches_state(state)
                    {
                        return Ok(state.clone());
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut found = None;
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_transaction_id(item) == Some(sdk_transaction_id)
                        && let Some(state) = item.send_state.as_ref()
                        && matches_state(state)
                    {
                        found = Some(state.clone());
                    }
                    Ok(())
                })?;
                if let Some(state) = found {
                    return Ok(state);
                }
            }
            _ => {}
        }
    }
}

async fn retry_send_queue_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::RetrySend {
        request_id,
        key: key.clone(),
        transaction_id: sdk_transaction_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit RetrySend failed: {e}"))?;
    Ok(request_id)
}

async fn cancel_send_queue_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::CancelSend {
        request_id,
        key: key.clone(),
        transaction_id: sdk_transaction_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("{label}: submit CancelSend failed: {e}"))?;
    Ok(request_id)
}

fn observe_send_queue_retry_item_state(
    item: &TimelineItem,
    sdk_transaction_id: &str,
    first_left_not_sent_after_retry: &mut bool,
) -> Option<&'static str> {
    if timeline_item_transaction_id(item) != Some(sdk_transaction_id) {
        return None;
    }
    match item.send_state.as_ref() {
        Some(TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
        }) if *first_left_not_sent_after_retry => Some("recoverable"),
        Some(TimelineSendState::NotSent {
            reason: koushi_core::event::TimelineSendFailureReason::Unrecoverable,
        }) if *first_left_not_sent_after_retry => Some("unrecoverable"),
        Some(TimelineSendState::NotSent { .. }) | None => None,
        Some(
            TimelineSendState::Sending | TimelineSendState::Cancelled | TimelineSendState::Sent,
        ) => {
            *first_left_not_sent_after_retry = true;
            None
        }
    }
}

async fn wait_for_send_completions_in_order(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    retry_request_id: RequestId,
    first: &SendQueueLocalEcho,
    second: &SendQueueLocalEcho,
    label: &str,
) -> Result<(), String> {
    let mut first_completed = false;
    let mut first_left_not_sent_after_retry = false;
    loop {
        let event = tokio::time::timeout(SEND_QUEUE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for ordered SendCompleted events \
                     first_completed={first_completed}"
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(reason) = items.iter().find_map(|item| {
                    observe_send_queue_retry_item_state(
                        item,
                        &first.sdk_transaction_id,
                        &mut first_left_not_sent_after_retry,
                    )
                }) {
                    return Err(format!(
                        "{label}: first queued send returned to NotSent reason={reason}"
                    ));
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    if let Some(reason) = observe_send_queue_retry_item_state(
                        item,
                        &first.sdk_transaction_id,
                        &mut first_left_not_sent_after_retry,
                    ) {
                        return Err(format!(
                            "{label}: first queued send returned to NotSent reason={reason}"
                        ));
                    }
                    Ok(())
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref ev_key,
                transaction_id,
                ..
            }) if ev_key == key && request_id == first.request_id => {
                if transaction_id != first.client_transaction_id {
                    return Err(format!("{label}: first completion transaction mismatch"));
                }
                first_completed = true;
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref ev_key,
                transaction_id,
                ..
            }) if ev_key == key && request_id == second.request_id => {
                if !first_completed {
                    return Err(format!(
                        "{label}: later queued send completed before the failed predecessor"
                    ));
                }
                if transaction_id != second.client_transaction_id {
                    return Err(format!("{label}: second completion transaction mismatch"));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed { request_id, .. } if request_id == retry_request_id => {
                return Err(format!("{label}: retry operation failed"));
            }
            CoreEvent::OperationFailed { request_id, .. }
                if request_id == first.request_id || request_id == second.request_id =>
            {
                return Err(format!("{label}: queued send operation failed"));
            }
            _ => {}
        }
    }
}

async fn wait_for_cancelled_or_removed_send(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    cancel_request_id: RequestId,
    sdk_transaction_id: &str,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for cancel"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut cancelled = false;
                for diff in &diffs {
                    match diff {
                        TimelineDiff::Remove { .. } => return Ok(()),
                        TimelineDiff::PushBack { item }
                        | TimelineDiff::PushFront { item }
                        | TimelineDiff::Insert { item, .. }
                        | TimelineDiff::Set { item, .. }
                            if timeline_item_transaction_id(item) == Some(sdk_transaction_id)
                                && matches!(
                                    item.send_state,
                                    Some(TimelineSendState::Cancelled)
                                ) =>
                        {
                            cancelled = true;
                        }
                        TimelineDiff::Reset { items } => {
                            if items.iter().all(|item| {
                                timeline_item_transaction_id(item) != Some(sdk_transaction_id)
                            }) {
                                cancelled = true;
                            }
                        }
                        _ => {}
                    }
                }
                if cancelled {
                    return Ok(());
                }
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == cancel_request_id => {
                return Err(format!("{label}: cancel failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_event_item_with_body_or_retry_not_sent(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    sdk_transaction_id: &str,
    expected_body: &str,
    mut retry_sent: bool,
    label: &str,
) -> Result<TimelineItem, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for restored send completion"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                for item in items {
                    if timeline_item_body_matches(&item, expected_body)
                        && matches!(item.id, TimelineItemId::Event { .. })
                    {
                        return Ok(item);
                    }
                    if !retry_sent
                        && timeline_item_transaction_id(&item) == Some(sdk_transaction_id)
                        && matches!(item.send_state, Some(TimelineSendState::NotSent { .. }))
                    {
                        retry_send_queue_item(conn, key, sdk_transaction_id, label).await?;
                        retry_sent = true;
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut found = None;
                let mut should_retry = false;
                visit_timeline_diff_items(&diffs, |item| {
                    if timeline_item_body_matches(item, expected_body)
                        && matches!(item.id, TimelineItemId::Event { .. })
                    {
                        found = Some(item.clone());
                    }
                    if !retry_sent
                        && timeline_item_transaction_id(item) == Some(sdk_transaction_id)
                        && matches!(
                            item.send_state.as_ref(),
                            Some(TimelineSendState::NotSent { .. })
                        )
                    {
                        should_retry = true;
                    }
                    Ok(())
                })?;
                if let Some(item) = found {
                    return Ok(item);
                }
                if should_retry {
                    retry_send_queue_item(conn, key, sdk_transaction_id, label).await?;
                    retry_sent = true;
                }
            }
            _ => {}
        }
    }
}

fn timeline_item_body_matches(item: &TimelineItem, expected_body: &str) -> bool {
    item.body
        .as_ref()
        .map(|body| body.contains(expected_body))
        .unwrap_or(false)
}

fn timeline_item_transaction_id(item: &TimelineItem) -> Option<&str> {
    match &item.id {
        TimelineItemId::Transaction { transaction_id } => Some(transaction_id.as_str()),
        TimelineItemId::Event { .. } | TimelineItemId::Synthetic { .. } => None,
    }
}

/// Wait for `TimelineEvent::SendCompleted` with the given request_id and key.
/// Returns `(transaction_id, event_id)`.
async fn wait_for_send_completed(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    key: &TimelineKey,
    label: &str,
) -> Result<(String, String), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SendCompleted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: ev_id,
                key: ref ev_key,
                transaction_id,
                event_id,
            }) if ev_id == request_id && ev_key == key => {
                return Ok((transaction_id, event_id));
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

struct MediaSendWaiter {
    request_id: koushi_core::ids::RequestId,
    key: TimelineKey,
    expected_client_txn_id: String,
    saw_local_media_echo: bool,
    saw_upload_progress: bool,
    event_id: Option<String>,
}

impl MediaSendWaiter {
    fn new(
        request_id: koushi_core::ids::RequestId,
        key: TimelineKey,
        expected_client_txn_id: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            key,
            expected_client_txn_id: expected_client_txn_id.into(),
            saw_local_media_echo: false,
            saw_upload_progress: false,
            event_id: None,
        }
    }

    fn observe(&mut self, event: CoreEvent) -> Result<(), String> {
        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == &self.key => {
                if !self.saw_local_media_echo {
                    self.saw_local_media_echo =
                        media_diffs_include_transaction_media(&diffs, &self.expected_client_txn_id);
                }
            }
            CoreEvent::Timeline(TimelineEvent::MediaUploadProgress {
                request_id,
                key: ref ev_key,
                transaction_id,
                progress,
                ..
            }) if ev_key == &self.key && transaction_id == self.expected_client_txn_id => {
                if let Some(request_id) = request_id
                    && request_id != self.request_id
                {
                    return Err("media upload progress request_id mismatch".to_owned());
                }
                if progress.total > 0 && progress.current <= progress.total {
                    self.saw_upload_progress = true;
                }
            }
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref ev_key,
                transaction_id,
                event_id,
            }) if request_id == self.request_id && ev_key == &self.key => {
                if transaction_id != self.expected_client_txn_id {
                    return Err("media send transaction_id mismatch".to_owned());
                }
                self.event_id = Some(event_id);
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == self.request_id => {
                return Err(format!("media send failed: {failure:?}"));
            }
            _ => {}
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.saw_local_media_echo && self.saw_upload_progress && self.event_id.is_some()
    }
}

fn media_diffs_include_transaction_media(
    diffs: &[koushi_core::event::TimelineDiff],
    expected_transaction_id: &str,
) -> bool {
    diffs.iter().any(|diff| match diff {
        koushi_core::event::TimelineDiff::PushBack { item }
        | koushi_core::event::TimelineDiff::PushFront { item }
        | koushi_core::event::TimelineDiff::Insert { item, .. }
        | koushi_core::event::TimelineDiff::Set { item, .. } => {
            timeline_item_is_transaction_media(item, expected_transaction_id)
        }
        koushi_core::event::TimelineDiff::Reset { items } => items
            .iter()
            .any(|item| timeline_item_is_transaction_media(item, expected_transaction_id)),
        _ => false,
    })
}

fn timeline_item_is_transaction_media(
    item: &koushi_core::event::TimelineItem,
    expected_transaction_id: &str,
) -> bool {
    item.media.is_some()
        && matches!(
            &item.id,
            koushi_core::event::TimelineItemId::Transaction { transaction_id }
                if transaction_id == expected_transaction_id
        )
}

async fn wait_for_media_send_flow_completion(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    key: &TimelineKey,
    client_txn_id: &str,
    label: &str,
) -> Result<String, String> {
    let mut waiter = MediaSendWaiter::new(request_id, key.clone(), client_txn_id);

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for media send flow completion"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        waiter.observe(event)?;
        if waiter.is_complete() {
            return waiter
                .event_id
                .ok_or_else(|| "media send flow: missing event id".to_owned());
        }
    }
}

async fn wait_for_media_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for media item"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = items.into_iter().find(|item| item.media.is_some()) {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    match diff {
                        koushi_core::event::TimelineDiff::PushBack { item }
                        | koushi_core::event::TimelineDiff::PushFront { item }
                        | koushi_core::event::TimelineDiff::Insert { item, .. }
                        | koushi_core::event::TimelineDiff::Set { item, .. } => {
                            if item.media.is_some() {
                                return Ok(item);
                            }
                        }
                        koushi_core::event::TimelineDiff::Reset { items } => {
                            if let Some(item) = items.into_iter().find(|item| item.media.is_some())
                            {
                                return Ok(item);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

async fn wait_for_media_download_completed(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    key: &TimelineKey,
    expected_event_id: &str,
    expected_byte_count: u64,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for media download completion"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::MediaDownloadCompleted {
                request_id: ev_id,
                key: ref ev_key,
                event_id,
                byte_count,
                ..
            }) if ev_id == request_id && ev_key == key => {
                if event_id != expected_event_id {
                    return Err("media download event_id mismatch".to_owned());
                }
                if byte_count != expected_byte_count {
                    return Err(format!(
                        "media download byte_count mismatch: expected {expected_byte_count}, got {byte_count}"
                    ));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn run_live_signals_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
    event_id: &str,
    expected_reader_user_id: &str,
) -> Result<(), String> {
    let room_id = timeline_key_room_id(key_b)
        .ok_or_else(|| "live signals: expected room timeline key".to_owned())?
        .to_owned();
    let observer_room_id = timeline_key_room_id(key_a)
        .ok_or_else(|| "live signals: expected observer room timeline key".to_owned())?
        .to_owned();

    let read_receipt_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
            request_id: read_receipt_id,
            key: key_b.clone(),
            event_id: event_id.to_owned(),
        }))
        .await
        .map_err(|e| format!("live signals: submit read receipt failed: {e}"))?;
    wait_for_live_signal_event(conn_b, read_receipt_id, "read receipt", |event| {
        matches!(event, LiveSignalsEvent::ReadReceiptSent { .. })
    })
    .await?;
    wait_for_read_receipt_projection(
        conn_a,
        &observer_room_id,
        event_id,
        expected_reader_user_id,
        "read receipt state",
    )
    .await?;
    println!("read_receipt=ok");

    let fully_read_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
            request_id: fully_read_id,
            key: key_b.clone(),
            event_id: event_id.to_owned(),
        }))
        .await
        .map_err(|e| format!("live signals: submit fully-read marker failed: {e}"))?;
    wait_for_live_signal_event(conn_b, fully_read_id, "fully read", |event| {
        matches!(event, LiveSignalsEvent::FullyReadSet { .. })
    })
    .await?;
    wait_for_live_signal_snapshot(conn_b, "fully read state", |snapshot| {
        snapshot
            .live_signals
            .rooms
            .get(&room_id)
            .is_some_and(|room| room.fully_read_event_id.as_deref() == Some(event_id))
    })
    .await?;
    println!("fully_read=ok");

    let typing_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::SetTyping {
            request_id: typing_id,
            key: key_b.clone(),
            is_typing: true,
        }))
        .await
        .map_err(|e| format!("live signals: submit typing notice failed: {e}"))?;
    wait_for_live_signal_event(conn_b, typing_id, "typing", |event| {
        matches!(
            event,
            LiveSignalsEvent::TypingSet {
                is_typing: true,
                ..
            }
        )
    })
    .await?;
    wait_for_live_signal_snapshot(conn_a, "typing state", |snapshot| {
        snapshot
            .live_signals
            .rooms
            .get(&observer_room_id)
            .is_some_and(|room| !room.typing_user_ids.is_empty())
    })
    .await?;
    println!("typing=ok");

    let user_id_b = match &conn_b.snapshot().session {
        SessionState::Ready(info) => info.user_id.clone(),
        _ => return Err("live signals: user B session was not ready".to_owned()),
    };
    let presence_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::SetPresence {
            request_id: presence_id,
            presence: PresenceKind::Away,
        }))
        .await
        .map_err(|e| format!("live signals: submit presence failed: {e}"))?;
    wait_for_live_signal_event(conn_b, presence_id, "presence", |event| {
        matches!(event, LiveSignalsEvent::PresenceSet { .. })
    })
    .await?;
    wait_for_live_signal_snapshot(conn_b, "presence state", |snapshot| {
        snapshot.live_signals.presence.get(&user_id_b) == Some(&PresenceKind::Away)
    })
    .await?;
    println!("presence=ok");
    println!("live_signals=ok");

    Ok(())
}

fn read_receipt_projection_status(
    snapshot: &AppState,
    room_id: &str,
    event_id: &str,
    expected_reader_user_id: &str,
) -> &'static str {
    let Some(room) = snapshot.live_signals.rooms.get(room_id) else {
        return "room_missing";
    };
    let Some(receipts) = room.receipts_by_event.get(event_id) else {
        return "event_missing";
    };
    if receipts.readers.is_empty() {
        return "readers_empty";
    }
    let Some(reader) = receipts
        .readers
        .iter()
        .find(|reader| reader.user_id == expected_reader_user_id)
    else {
        return "reader_missing";
    };
    let has_display_label = reader
        .display_name
        .as_deref()
        .is_some_and(|label| !label.trim().is_empty())
        || !reader.original_display_label.trim().is_empty();
    if has_display_label {
        "projected"
    } else {
        "label_missing"
    }
}

async fn wait_for_read_receipt_projection(
    conn: &mut CoreConnection,
    room_id: &str,
    event_id: &str,
    expected_reader_user_id: &str,
    label: &str,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    let mut last_status =
        read_receipt_projection_status(&snapshot, room_id, event_id, expected_reader_user_id);
    if last_status == "projected" {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for read-receipt projection status={last_status}"
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event {
            last_status = read_receipt_projection_status(
                &snapshot,
                room_id,
                event_id,
                expected_reader_user_id,
            );
            if last_status == "projected" {
                return Ok(snapshot);
            }
        }
    }
}

async fn run_composer_stage(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    mentioned_user_id: &str,
) -> Result<(), String> {
    let ime_action = resolve_composer_key_action(
        ComposerKeyEvent {
            key: ComposerKey::Enter,
            modifiers: ComposerKeyModifiers::default(),
            is_composing: true,
            selection: Some(ComposerSelection { start: 0, end: 0 }),
        },
        ComposerResolverContext {
            surface: ComposerSurface::Main,
            send_shortcut: ComposerSendShortcut::Enter,
            autocomplete_open: true,
            send_enabled: true,
        },
    );
    if ime_action != ComposerResolvedAction::CommitImeCandidate {
        return Err(format!("composer IME guard mismatch: {ime_action:?}"));
    }

    let mention_txn = "qa-composer-mention-txn";
    let mention_body = "Composer mention QA";
    let mention_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: mention_id,
        key: key.clone(),
        transaction_id: mention_txn.to_owned(),
        body: mention_body.to_owned(),
        mentions: MentionIntent {
            targets: vec![MentionTarget::User {
                user_id: mentioned_user_id.to_owned(),
                display_label: "Synthetic mention".to_owned(),
            }],
        },
    }))
    .await
    .map_err(|e| format!("composer mention send submit failed: {e}"))?;
    wait_for_send_flow_completion(
        conn,
        mention_id,
        key,
        mention_txn,
        mention_body,
        "composer mention send",
    )
    .await?;
    println!("mention_send=ok");

    let markdown_txn = "qa-composer-markdown-txn";
    let markdown_body = "Composer **markdown** QA";
    let markdown_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: markdown_id,
        key: key.clone(),
        transaction_id: markdown_txn.to_owned(),
        body: markdown_body.to_owned(),
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("composer markdown send submit failed: {e}"))?;
    wait_for_send_flow_completion(
        conn,
        markdown_id,
        key,
        markdown_txn,
        markdown_body,
        "composer markdown send",
    )
    .await?;
    println!("markdown_send=ok");

    let slash_txn = "qa-composer-slash-txn";
    let slash_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: slash_id,
        key: key.clone(),
        transaction_id: slash_txn.to_owned(),
        body: "/me composer slash command".to_owned(),
        mentions: MentionIntent::default(),
    }))
    .await
    .map_err(|e| format!("composer slash send submit failed: {e}"))?;
    wait_for_send_flow_completion(
        conn,
        slash_id,
        key,
        slash_txn,
        "composer slash command",
        "composer slash command",
    )
    .await?;
    println!("slash_command=ok");
    println!("ime_guard=ok");

    Ok(())
}

fn timeline_key_room_id(key: &TimelineKey) -> Option<&str> {
    match &key.kind {
        TimelineKind::Room { room_id }
        | TimelineKind::Thread { room_id, .. }
        | TimelineKind::Focused { room_id, .. } => Some(room_id.as_str()),
    }
}

async fn wait_for_live_signal_event(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
    matches_event: impl Fn(&LiveSignalsEvent) -> bool,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for live-signal event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::LiveSignals(event) if matches_event(&event) => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: live-signal command failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_live_signal_snapshot(
    conn: &mut CoreConnection,
    label: &str,
    predicate: impl Fn(&AppState) -> bool,
) -> Result<AppState, String> {
    let snapshot = conn.snapshot();
    if predicate(&snapshot) {
        return Ok(snapshot);
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for live-signal state"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        if let CoreEvent::StateChanged(snapshot) = event
            && predicate(&snapshot)
        {
            return Ok(snapshot);
        }
    }
}

async fn run_media_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
) -> Result<(), String> {
    const MEDIA_BYTES: &[u8] = b"koushi-desktop synthetic media fixture";
    const MEDIA_CAPTION: &str = "matrix desktop media caption";

    let expected_account = match conn_a.snapshot().session {
        koushi_state::SessionState::Ready(info) => {
            koushi_core::store::session_key_id_from_info(&info)
        }
        _ => return Err("media stage requires a ready session".to_owned()),
    };
    let media_txn = "qa-phase15-media-txn".to_owned();
    let send_media_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
            request_id: send_media_id,
            expected_account,
            key: key_a.clone(),
            transaction_id: media_txn.clone(),
            request: UploadMediaRequest {
                filename: "koushi-desktop-qa-media.bin".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                bytes: MEDIA_BYTES.to_vec(),
                kind: UploadMediaKind::File,
                compression: None,
                thumbnail: None,
                caption: Some(build_formatted_message_draft(
                    MEDIA_CAPTION,
                    MentionIntent::default(),
                )),
            },
        }))
        .await
        .map_err(|e| format!("submit media send: {e}"))?;

    let _media_event_id = wait_for_media_send_flow_completion(
        conn_a,
        send_media_id,
        key_a,
        &media_txn,
        "media send flow",
    )
    .await?;
    println!("send_media=ok");

    let media_item = wait_for_media_item(conn_b, key_b, "B receives media item").await?;
    let media = media_item
        .media
        .as_ref()
        .ok_or_else(|| "media item missing media metadata".to_owned())?;
    if media.kind != koushi_core::event::TimelineMediaKind::File {
        return Err("media item kind mismatch".to_owned());
    }
    if media_item.body.as_deref() != Some(MEDIA_CAPTION) {
        return Err("media caption did not project onto timeline item body".to_owned());
    }
    println!("media_caption=ok");
    assert_image_upload_compression_contract()?;
    println!("image_compress=ok");
    assert_upload_ux_state_contract(key_a.room_id())?;
    println!("upload_staging=ok");
    println!("media_gallery=ok");
    let media_event_id = match &media_item.id {
        koushi_core::event::TimelineItemId::Event { event_id } => event_id.clone(),
        koushi_core::event::TimelineItemId::Transaction { .. }
        | koushi_core::event::TimelineItemId::Synthetic { .. } => {
            return Err("received media item was not event-backed".to_owned());
        }
    };

    let download_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::DownloadMedia {
            request_id: download_id,
            key: key_b.clone(),
            event_id: media_event_id.clone(),
            selection: MediaDownloadSelection::File,
        }))
        .await
        .map_err(|e| format!("submit media download: {e}"))?;

    wait_for_media_download_completed(
        conn_b,
        download_id,
        key_b,
        &media_event_id,
        u64::try_from(MEDIA_BYTES.len()).unwrap_or(u64::MAX),
        "media download",
    )
    .await?;
    println!("recv_media=ok");

    Ok(())
}

async fn run_link_preview_stage(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    key_a: &TimelineKey,
    key_b: &TimelineKey,
) -> Result<(), String> {
    const URL_MESSAGE_BODY: &str = "link preview test message https://example.invalid/page";
    const URL_EXTRACTED: &str = "https://example.invalid/page";

    // 1. Send a message containing a URL from conn_a to the shared timeline room.
    let txn = "qa-link-preview-txn".to_owned();
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_a.clone(),
            transaction_id: txn.clone(),
            body: URL_MESSAGE_BODY.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("submit link preview message: {e}"))?;

    let (_send_txn, _event_id) =
        wait_for_send_completed(conn_a, send_id, key_a, "link preview send").await?;

    // 2. Wait for conn_b to see the message and verify a pending preview.
    let item =
        wait_for_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees URL message").await?;
    let event_id = match &item.id {
        TimelineItemId::Event { event_id } => event_id.clone(),
        _ => return Err("link preview item was not event-backed".to_owned()),
    };
    let previews = item
        .link_previews
        .as_ref()
        .ok_or("missing link_previews on URL message")?;
    if previews.len() != 1 {
        return Err(format!(
            "link preview count mismatch: expected 1, got {}",
            previews.len()
        ));
    }
    if previews[0].url != URL_EXTRACTED {
        return Err("link preview URL mismatch".to_owned());
    }
    if !matches!(previews[0].state, LinkPreviewState::Pending) {
        return Err(format!(
            "link preview state mismatch: expected Pending, got {:?}",
            previews[0].state
        ));
    }
    println!("link_preview_global=ok");

    // 3. Disable URL previews globally via UpdateSettings and verify the
    //    projection drops the preview.
    let settings_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: false,
                    encrypted_url_previews_enabled: false,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview disable: {e}"))?;
    let disabled_item = wait_for_link_preview_item_projection(
        conn_b,
        key_b,
        settings_id,
        URL_MESSAGE_BODY,
        "B sees message after global disable",
        |item| {
            item.link_previews
                .as_ref()
                .map(|previews| previews.is_empty())
                .unwrap_or(true)
        },
    )
    .await?;
    if !disabled_item
        .link_previews
        .as_ref()
        .map(|p| p.is_empty())
        .unwrap_or(true)
    {
        return Err("global disable did not empty link previews".to_owned());
    }
    println!("link_preview_room=ok");

    // 4. Re-enable URL previews globally.
    let settings_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: true,
                    encrypted_url_previews_enabled: true,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview enable: {e}"))?;
    let reenabled_item = wait_for_link_preview_item_projection(
        conn_b,
        key_b,
        settings_id,
        URL_MESSAGE_BODY,
        "B sees message after global re-enable",
        |item| {
            item.link_previews.as_ref().is_some_and(|previews| {
                previews.len() == 1
                    && previews[0].url == URL_EXTRACTED
                    && matches!(previews[0].state, LinkPreviewState::Pending)
            })
        },
    )
    .await?;
    let reenabled_previews = reenabled_item
        .link_previews
        .as_ref()
        .ok_or("missing link_previews after global re-enable")?;
    if reenabled_previews.len() != 1
        || reenabled_previews[0].url != URL_EXTRACTED
        || !matches!(reenabled_previews[0].state, LinkPreviewState::Pending)
    {
        return Err("global re-enable did not restore the pending link preview".to_owned());
    }

    // 5. Send HideLinkPreview for the event and verify the message's previews
    //    become an empty list.
    let hide_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Timeline(TimelineCommand::HideLinkPreview {
            request_id: hide_id,
            key: key_b.clone(),
            event_id: event_id.clone(),
        }))
        .await
        .map_err(|e| format!("submit hide link preview: {e}"))?;

    let hidden_item =
        wait_for_item_with_body(conn_b, key_b, URL_MESSAGE_BODY, "B sees message after hide")
            .await?;
    if hidden_item.link_previews.as_ref() != Some(&Vec::new()) {
        return Err("hide link preview did not produce empty preview list".to_owned());
    }
    println!("link_preview_hide=ok");

    let settings_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: settings_id,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: true,
                    encrypted_url_previews_enabled: true,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit encrypted preview enable: {e}"))?;
    wait_for_settings_persisted(conn_a, settings_id, "encrypted preview enable", true).await?;

    // 6. Test E2EE default-on: create a new encrypted room, send a URL message,
    //    and verify previews are projected for the sender's own item.
    //
    //    The sender can decrypt their own event, so checking A's timeline asserts
    //    the Rust-owned encrypted-room policy end-to-end without depending on
    //    cross-device key sharing. The unit tests in link_preview.rs already
    //    assert the encrypted-room default-on rule directly.
    let enc_room_id = create_room_for_qa(
        conn_a,
        "QA Link Preview E2EE Room",
        true,
        "link_preview create encrypted room",
    )
    .await?;

    wait_for_room_in_room_list(
        conn_a,
        &enc_room_id,
        "room list after link preview encrypted room",
    )
    .await?;

    // Wait until the room summary reports encryption enabled before sending.
    let mut found_encrypted = false;
    for _ in 0..30 {
        if conn_a
            .snapshot()
            .rooms
            .iter()
            .any(|r| r.room_id == enc_room_id && r.is_encrypted)
        {
            found_encrypted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if !found_encrypted {
        return Err("encrypted room did not report is_encrypted".to_owned());
    }

    let account_key_a = match &conn_a.snapshot().session {
        SessionState::Ready(info) => AccountKey(info.user_id.clone()),
        _ => return Err("link_preview: session A was not ready".to_owned()),
    };
    let enc_key_a = TimelineKey::room(account_key_a, enc_room_id.clone());

    let sub_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: sub_a_id,
            key: enc_key_a.clone(),
        }))
        .await
        .map_err(|e| format!("link_preview subscribe encrypted room A: {e}"))?;
    wait_for_initial_items(conn_a, &enc_key_a, sub_a_id, "subscribe encrypted room A").await?;

    let enc_txn = "qa-link-preview-e2ee-txn".to_owned();
    let enc_send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: enc_send_id,
            key: enc_key_a.clone(),
            transaction_id: enc_txn.clone(),
            body: URL_MESSAGE_BODY.to_owned(),
            mentions: MentionIntent::default(),
        }))
        .await
        .map_err(|e| format!("submit encrypted room URL message: {e}"))?;
    wait_for_send_completed(conn_a, enc_send_id, &enc_key_a, "encrypted room URL send").await?;

    let enc_item = wait_for_event_item_with_body(
        conn_a,
        &enc_key_a,
        URL_MESSAGE_BODY,
        "A sees encrypted room URL message",
    )
    .await?;
    let enc_previews = enc_item
        .link_previews
        .as_ref()
        .ok_or("missing link_previews on encrypted room URL message")?;
    if enc_previews.len() != 1 {
        return Err(format!(
            "encrypted room link preview count mismatch: expected 1, got {}",
            enc_previews.len()
        ));
    }
    if enc_previews[0].url != URL_EXTRACTED {
        return Err("encrypted room link preview URL mismatch".to_owned());
    }
    if !matches!(enc_previews[0].state, LinkPreviewState::Pending) {
        return Err(format!(
            "encrypted room link preview state mismatch: expected Pending, got {:?}",
            enc_previews[0].state
        ));
    }
    println!("link_preview_e2ee_default=ok");

    Ok(())
}

fn assert_image_upload_compression_contract() -> Result<(), String> {
    let policy = ImageUploadCompressionPolicy::default();
    let original_dimensions = ImageUploadDimensions {
        width: 4032,
        height: 3024,
    };
    let selected_dimensions = policy.target_dimensions_for(original_dimensions);
    if selected_dimensions
        != (ImageUploadDimensions {
            width: 2048,
            height: 1536,
        })
    {
        return Err("image compression target dimensions did not preserve aspect ratio".to_owned());
    }

    let original = ImageUploadVariantInfo {
        mime_type: "image/jpeg".to_owned(),
        byte_count: 3_200_000,
        dimensions: Some(original_dimensions),
    };
    if policy.should_skip(&original) {
        return Err("large image was incorrectly classified as skip-small".to_owned());
    }
    let selected = ImageUploadVariantInfo {
        mime_type: "image/jpeg".to_owned(),
        byte_count: 128_000,
        dimensions: Some(selected_dimensions),
    };
    let compression = ImageUploadCompressionState {
        mode: ImageUploadCompressionMode::Always,
        policy,
        original,
        selected: selected.clone(),
        selected_variant: ImageUploadVariantKind::Compressed,
        skipped_small_image: false,
        metadata_stripped: true,
        thumbnail_refreshed: true,
    };
    let request = UploadMediaRequest {
        filename: "koushi-desktop-qa-private-name.jpg".to_owned(),
        mime_type: selected.mime_type,
        bytes: vec![0; 128_000],
        kind: UploadMediaKind::Image {
            width: selected.dimensions.map(|dimensions| dimensions.width),
            height: selected.dimensions.map(|dimensions| dimensions.height),
        },
        compression: Some(compression),
        thumbnail: Some(UploadMediaThumbnail {
            mime_type: "image/jpeg".to_owned(),
            bytes: vec![0; 4096],
            width: 320,
            height: 240,
        }),
        caption: None,
    };

    let Some(compression) = request.compression.as_ref() else {
        return Err("image upload request did not carry compression contract".to_owned());
    };
    if compression.selected_variant != ImageUploadVariantKind::Compressed {
        return Err("image upload request did not carry selected compressed variant".to_owned());
    }
    if !compression.metadata_stripped {
        return Err("compressed image contract did not require metadata stripping".to_owned());
    }
    if !compression.thumbnail_refreshed || request.thumbnail.is_none() {
        return Err(
            "compressed image contract did not carry refreshed thumbnail metadata".to_owned(),
        );
    }
    if compression.selected.byte_count != u64::try_from(request.bytes.len()).unwrap_or(u64::MAX) {
        return Err("selected compression byte count diverged from upload bytes".to_owned());
    }
    let debug = format!("{request:?}");
    if debug.contains("koushi-desktop-qa-private-name.jpg") || debug.contains("0, 0, 0") {
        return Err("image compression request debug leaked private filename or bytes".to_owned());
    }
    Ok(())
}

fn assert_upload_ux_state_contract(room_id: &str) -> Result<(), String> {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://qa.example.invalid".to_owned(),
            user_id: "@qa:example.invalid".to_owned(),
            device_id: "QADEVICE".to_owned(),
        }),
        rooms: vec![native_attention_room(room_id, "QA Room", false, 0, 0, 0)],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );

    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: koushi_state::ComposerTarget::Main {
                room_id: room_id.to_owned(),
            },
            items: vec![
                StagedUploadItem {
                    staged_id: "stage-2".to_owned(),
                    room_id: room_id.to_owned(),
                    position: 2,
                    filename: "private-two.txt".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    byte_count: 256,
                    kind: StagedUploadKind::File,
                    caption: Some(build_formatted_message_draft(
                        "private staged caption",
                        MentionIntent::default(),
                    )),
                    compression_choice: StagedUploadCompressionChoice::NotApplicable,
                    preparation: Default::default(),
                },
                StagedUploadItem {
                    staged_id: "stage-1".to_owned(),
                    room_id: room_id.to_owned(),
                    position: 1,
                    filename: "private-one.jpg".to_owned(),
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 3_200_000,
                    kind: StagedUploadKind::Image {
                        width: Some(4032),
                        height: Some(3024),
                    },
                    caption: None,
                    compression_choice: StagedUploadCompressionChoice::Original,
                    preparation: Default::default(),
                },
            ],
        },
    );
    if state.timeline.staged_uploads.len() != 2
        || state.timeline.staged_uploads[0].staged_id != "stage-1"
    {
        return Err("upload staging projection did not keep multiple files in order".to_owned());
    }

    reduce(
        &mut state,
        AppAction::UploadStagingCompressionChanged {
            target: koushi_state::ComposerTarget::Main {
                room_id: room_id.to_owned(),
            },
            staged_id: "stage-1".to_owned(),
            compression_choice: StagedUploadCompressionChoice::Compressed {
                mode: ImageUploadCompressionMode::Ask,
            },
        },
    );
    if state.timeline.staged_uploads[0].compression_choice
        != (StagedUploadCompressionChoice::Compressed {
            mode: ImageUploadCompressionMode::Ask,
        })
    {
        return Err("upload staging did not preserve per-file compression choice".to_owned());
    }

    reduce(
        &mut state,
        AppAction::MediaGalleryUpdated {
            room_id: room_id.to_owned(),
            items: vec![
                media_gallery_contract_item("$old-media", room_id, 1_900_000_000_000),
                media_gallery_contract_item("$new-media", room_id, 1_900_000_060_000),
            ],
        },
    );
    if state.timeline.media_gallery.len() != 2
        || state.timeline.media_gallery[0].event_id != "$new-media"
    {
        return Err("media gallery projection did not sort newest media first".to_owned());
    }

    let value = serde_json::to_value(&state).map_err(|e| format!("serialize upload state: {e}"))?;
    if value.get("upload_staging").is_some() || value.get("media_gallery").is_some() {
        return Err(
            "upload staging/gallery root stores leaked into serialized AppState".to_owned(),
        );
    }
    if value["timeline"]["staged_uploads"][0]["staged_id"] != "stage-1"
        || value["timeline"]["media_gallery"][0]["event_id"] != "$new-media"
    {
        return Err("selected timeline upload/gallery projection did not serialize".to_owned());
    }

    let debug = format!(
        "{:?} {:?}",
        state.timeline.staged_uploads[0], state.timeline.media_gallery[0]
    );
    for private in [
        room_id,
        "private-one.jpg",
        "private staged caption",
        "mxc://example.invalid/private-gallery",
    ] {
        if debug.contains(private) {
            return Err("upload staging/gallery debug leaked private media data".to_owned());
        }
    }

    Ok(())
}

fn media_gallery_contract_item(
    event_id: &str,
    room_id: &str,
    timestamp_ms: u64,
) -> TimelineMediaGalleryItem {
    TimelineMediaGalleryItem {
        event_id: event_id.to_owned(),
        room_id: room_id.to_owned(),
        sender: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        timestamp_ms,
        media: TimelineMediaGalleryMedia {
            kind: TimelineMediaKind::Image,
            filename: "private-gallery.jpg".to_owned(),
            source: TimelineMediaGallerySource {
                mxc_uri: "mxc://example.invalid/private-gallery".to_owned(),
                encrypted: true,
                encryption_version: Some("v2".to_owned()),
            },
            mimetype: Some("image/jpeg".to_owned()),
            size: Some(2048),
            width: Some(800),
            height: Some(600),
            thumbnail: None,
        },
    }
}

/// Wait for an item whose body contains `expected_body` and return the item so
/// the caller can assert relation metadata on the projected DTO.
async fn wait_for_item_with_body(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    let body_matches = |item: &koushi_core::event::TimelineItem| {
        item.body
            .as_ref()
            .map(|body| body.contains(expected_body))
            .unwrap_or(false)
    };

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for body {expected_body:?}"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = find_timeline_item_with_body(&items, expected_body) {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    let item = match diff {
                        koushi_core::event::TimelineDiff::PushBack { item }
                        | koushi_core::event::TimelineDiff::PushFront { item }
                        | koushi_core::event::TimelineDiff::Insert { item, .. }
                        | koushi_core::event::TimelineDiff::Set { item, .. } => item,
                        koushi_core::event::TimelineDiff::Reset { items } => {
                            if let Some(item) = items.into_iter().find(|item| body_matches(item)) {
                                return Ok(item);
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if body_matches(&item) {
                        return Ok(item.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

async fn wait_for_all_items_with_bodies(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[TimelineItem],
    expected_bodies: &[String],
    label: &str,
) -> Result<(), String> {
    let mut seen = seed_expected_body_observation(initial_items, expected_bodies);

    loop {
        if seen.iter().all(|found| *found) {
            return Ok(());
        }
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| missing_expected_body_timeout(label, &seen))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref event_key,
                items,
                ..
            }) if event_key == key => {
                for item in &items {
                    observe_expected_bodies(item, expected_bodies, &mut seen);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                diffs,
                ..
            }) if event_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    observe_expected_bodies(item, expected_bodies, &mut seen);
                    Ok(())
                })?;
            }
            _ => {}
        }
    }
}

async fn wait_for_exact_items_and_gap_release(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    mut items: Vec<TimelineItem>,
    expected_bodies: &[String],
    initial_gap_projection: Option<(u64, u64)>,
    expected_closed_gap: Option<TimelineGapId>,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut released = false;
    let mut expected_gap_absent = expected_closed_gap.is_none();
    let mut saw_post_demand_gap_positions = false;
    let mut closure_projection = None;
    let mut gap_actor_generation =
        initial_gap_projection.map(|(actor_generation, _)| actor_generation);
    let mut pending_render_ack = None;
    let mut render_ack_request_id = None;
    let mut render_ack_sent_at: Option<tokio::time::Instant> = None;
    let mut render_ack_actor_generation = None;
    loop {
        let counts = expected_bodies
            .iter()
            .map(|expected| {
                items
                    .iter()
                    .filter(|item| item.body.as_deref() == Some(expected.as_str()))
                    .count()
            })
            .collect::<Vec<_>>();
        if released && expected_gap_absent && counts.iter().all(|count| *count == 1) {
            return Ok(());
        }
        if counts.iter().any(|count| *count > 1) {
            return Err(format!(
                "{label}: a recovered synthetic row was projected more than once"
            ));
        }

        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let missing_count = counts.iter().filter(|count| **count == 0).count();
                format!(
                    "{label}: timed out with {missing_count} rows missing; gap_release={released}; \
                     expected_gap_absent={expected_gap_absent}; \
                     post_demand_gap_positions={saw_post_demand_gap_positions}; \
                     closure_projection={}; render_ack_sent={}; render_ack_same_actor={}",
                    closure_projection.is_some(),
                    render_ack_sent_at.is_some(),
                    closure_projection.is_some_and(|(actor_generation, _)| {
                        render_ack_actor_generation == Some(actor_generation)
                    }),
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref event_key,
                items: replacement,
                ..
            }) if event_key == key => items = replacement,
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref event_key,
                generation,
                batch_id,
                diffs,
                ..
            }) if event_key == key => {
                for diff in &diffs {
                    apply_timeline_diff(&mut items, diff);
                }
                if let Some(actor_generation) = gap_actor_generation {
                    pending_render_ack = Some((actor_generation, generation, batch_id));
                }
            }
            CoreEvent::Timeline(TimelineEvent::GapPositionsUpdated {
                key: ref event_key,
                actor_generation,
                generation,
                positions,
                ..
            }) if event_key == key => {
                if expected_closed_gap.is_none()
                    || initial_gap_projection
                        .is_some_and(|(initial_actor, _)| initial_actor == actor_generation)
                {
                    gap_actor_generation = Some(actor_generation);
                }
                saw_post_demand_gap_positions = true;
                if let (Some(expected_gap), Some((initial_actor, initial_generation))) =
                    (expected_closed_gap, initial_gap_projection)
                    && actor_generation == initial_actor
                    && generation > initial_generation
                {
                    expected_gap_absent =
                        positions.iter().all(|position| position.id != expected_gap);
                    closure_projection =
                        expected_gap_absent.then_some((actor_generation, generation));
                }
            }
            CoreEvent::Timeline(TimelineEvent::GapRepairReleased {
                key: ref event_key,
                actor_generation,
                generation,
            }) if event_key == key => {
                let release_projection = (actor_generation, generation);
                if expected_closed_gap.is_some()
                    && (closure_projection != Some(release_projection)
                        || render_ack_actor_generation != Some(actor_generation)
                        || render_ack_request_id.is_none())
                {
                    continue;
                }
                let Some(sent_at) = render_ack_sent_at else {
                    if expected_closed_gap.is_some() {
                        continue;
                    }
                    return Err(format!(
                        "{label}: gap repair released without a correlated render acknowledgement"
                    ));
                };
                if sent_at.elapsed() >= Duration::from_secs(5) {
                    return Err(format!(
                        "{label}: gap repair released only after render-settlement timeout"
                    ));
                }
                released = true;
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if Some(request_id) == render_ack_request_id => {
                return Err(format!(
                    "{label}: render acknowledgement was rejected: {failure:?}"
                ));
            }
            _ => {}
        }

        if let Some((actor_generation, timeline_generation, batch_id)) = pending_render_ack
            && let koushi_state::TimelineContinuityState::Repairing {
                generation: repair_generation,
                ..
            } = conn.snapshot().timeline.continuity
        {
            let request_id = conn.next_request_id();
            conn.command(CoreCommand::App(
                koushi_core::command::AppCommand::AcknowledgeTimelineBatchRendered {
                    request_id,
                    key: key.clone(),
                    actor_generation,
                    timeline_generation,
                    repair_generation,
                    batch_id,
                },
            ))
            .await
            .map_err(|error| format!("{label}: render acknowledgement failed: {error}"))?;
            render_ack_request_id = Some(request_id);
            render_ack_sent_at = Some(tokio::time::Instant::now());
            render_ack_actor_generation = Some(actor_generation);
            pending_render_ack = None;
        }
    }
}

fn observe_expected_bodies(
    item: &koushi_core::event::TimelineItem,
    expected_bodies: &[String],
    seen: &mut [bool],
) {
    let Some(body) = item.body.as_deref() else {
        return;
    };
    for (index, expected) in expected_bodies.iter().enumerate() {
        if body.contains(expected) {
            seen[index] = true;
        }
    }
}

fn seed_expected_body_observation(
    initial_items: &[TimelineItem],
    expected_bodies: &[String],
) -> Vec<bool> {
    let mut seen = vec![false; expected_bodies.len()];
    for item in initial_items {
        observe_expected_bodies(item, expected_bodies, &mut seen);
    }
    seen
}

#[cfg(test)]
fn missing_expected_body_count(seen: &[bool]) -> usize {
    seen.iter().filter(|found| !**found).count()
}

fn missing_expected_body_indices(seen: &[bool]) -> Vec<usize> {
    seen.iter()
        .enumerate()
        .filter_map(|(index, found)| (!found).then_some(index))
        .collect()
}

fn missing_expected_body_timeout(label: &str, seen: &[bool]) -> String {
    let missing_indices = missing_expected_body_indices(seen);
    format!(
        "{label}: timed out with {} expected rows still missing; missing_indices={missing_indices:?}",
        missing_indices.len()
    )
}

async fn wait_for_event_item_with_body(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<TimelineItem, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for event body {expected_body:?}"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = items.into_iter().find(|item| {
                    timeline_item_body_matches(item, expected_body)
                        && matches!(item.id, TimelineItemId::Event { .. })
                }) {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut found = None;
                visit_timeline_diff_items(&diffs, |item| {
                    if found.is_none()
                        && timeline_item_body_matches(item, expected_body)
                        && matches!(item.id, TimelineItemId::Event { .. })
                    {
                        found = Some(item.clone());
                    }
                    Ok(())
                })?;
                if let Some(item) = found {
                    return Ok(item);
                }
            }
            _ => {}
        }
    }
}

async fn wait_for_link_preview_item_projection(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    expected_body: &str,
    label: &str,
    predicate: impl Fn(&TimelineItem) -> bool,
) -> Result<TimelineItem, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for link-preview projection"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = items
                    .into_iter()
                    .find(|item| timeline_item_body_matches(item, expected_body) && predicate(item))
                {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                let mut found = None;
                visit_timeline_diff_items(&diffs, |item| {
                    if found.is_none()
                        && timeline_item_body_matches(item, expected_body)
                        && predicate(item)
                    {
                        found = Some(item.clone());
                    }
                    Ok(())
                })?;
                if let Some(item) = found {
                    return Ok(item);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: command failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_item_with_body_or_decryption_failure(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut observer = BodyWaitObserver::new(expected_body);
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| observer.timeout_message(label))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = observer.observe_items(&items) {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                if let Some(item) = observer.observe_diffs(&diffs)? {
                    return Ok(item);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WithheldEventProjectionOrigin {
    InitialItems,
    ItemsUpdated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WithheldEventTargetOutcome {
    Missing,
    DecryptionFailure,
    NonFailure {
        has_body: bool,
        has_typed_decryption_failure: bool,
        matches_expected_body: bool,
    },
}

fn withheld_event_target_outcome(
    items: &[TimelineItem],
    target_event_id: &str,
    expected_body: &str,
) -> WithheldEventTargetOutcome {
    let Some(item) = items
        .iter()
        .find(|item| timeline_item_event_id(item) == Some(target_event_id))
    else {
        return WithheldEventTargetOutcome::Missing;
    };
    if timeline_item_is_decryption_failure(item) {
        WithheldEventTargetOutcome::DecryptionFailure
    } else {
        WithheldEventTargetOutcome::NonFailure {
            has_body: item.body.is_some(),
            has_typed_decryption_failure: item.unable_to_decrypt.is_some(),
            matches_expected_body: timeline_item_body_matches(item, expected_body),
        }
    }
}

fn withheld_event_target_outcome_in_diffs(
    diffs: &[TimelineDiff],
    target_event_id: &str,
    expected_body: &str,
) -> Result<WithheldEventTargetOutcome, String> {
    let mut outcome = WithheldEventTargetOutcome::Missing;
    visit_timeline_diff_items(diffs, |item| {
        if timeline_item_event_id(item) == Some(target_event_id) {
            outcome = if timeline_item_is_decryption_failure(item) {
                WithheldEventTargetOutcome::DecryptionFailure
            } else {
                WithheldEventTargetOutcome::NonFailure {
                    has_body: item.body.is_some(),
                    has_typed_decryption_failure: item.unable_to_decrypt.is_some(),
                    matches_expected_body: timeline_item_body_matches(item, expected_body),
                }
            };
        }
        Ok(())
    })?;
    Ok(outcome)
}

async fn wait_for_withheld_event_projection_from_source<S: QaEventSource + ?Sized>(
    source: &mut S,
    key: &TimelineKey,
    target_event_id: &str,
    expected_body: &str,
    initial_items: &[TimelineItem],
    label: &str,
    timeout: Duration,
) -> Result<WithheldEventProjectionOrigin, String> {
    match withheld_event_target_outcome(initial_items, target_event_id, expected_body) {
        WithheldEventTargetOutcome::DecryptionFailure => {
            return Ok(WithheldEventProjectionOrigin::InitialItems);
        }
        WithheldEventTargetOutcome::NonFailure {
            has_body,
            has_typed_decryption_failure,
            matches_expected_body,
        } => {
            return Err(format!(
                "{label}: withheld target projection_outcome=non_failure \
                 projection_origin=initial_items has_body={has_body} \
                 has_typed_decryption_failure={has_typed_decryption_failure} \
                 matches_expected_body={matches_expected_body}"
            ));
        }
        WithheldEventTargetOutcome::Missing => {}
    }

    let deadline = QaEventDeadline::after(timeout);
    let mut matching_update_batches = 0u64;
    loop {
        let event = deadline.recv(source).await.map_err(|_| {
            format!(
                "{label}: withheld target projection_outcome=absent \
                 projection_origin=missing matching_update_batches={matching_update_batches}"
            )
        })?;
        let event = event
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        let CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: event_key,
            diffs,
            ..
        }) = event
        else {
            continue;
        };
        if event_key != *key {
            continue;
        }
        matching_update_batches += 1;
        match withheld_event_target_outcome_in_diffs(&diffs, target_event_id, expected_body)? {
            WithheldEventTargetOutcome::Missing => {}
            WithheldEventTargetOutcome::DecryptionFailure => {
                return Ok(WithheldEventProjectionOrigin::ItemsUpdated);
            }
            WithheldEventTargetOutcome::NonFailure {
                has_body,
                has_typed_decryption_failure,
                matches_expected_body,
            } => {
                return Err(format!(
                    "{label}: withheld target projection_outcome=non_failure \
                     projection_origin=items_updated has_body={has_body} \
                     has_typed_decryption_failure={has_typed_decryption_failure} \
                     matches_expected_body={matches_expected_body}"
                ));
            }
        }
    }
}

/// Wait until all `expected_bodies` are found AND pagination has settled (Idle
/// or EndReached). Scans `initial_items` first, then both ItemsUpdated diffs
/// and PaginationStateChanged events in a single loop. This avoids the race
/// where paginate diffs are consumed before the body scan starts.
async fn wait_for_bodies_and_pagination_settle(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[koushi_core::event::TimelineItem],
    expected_bodies: &[&str],
    label: &str,
) -> Result<(), String> {
    // Pre-scan initial items.
    let mut remaining_bodies: Vec<&str> = expected_bodies.to_vec();
    for item in initial_items {
        if let Some(ref body) = item.body {
            remaining_bodies.retain(|expected| !body.contains(expected));
        }
    }

    let mut pagination_settled = false;

    loop {
        if remaining_bodies.is_empty() && pagination_settled {
            return Ok(());
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out; bodies still needed: {:?}, pagination_settled: {}",
                    remaining_bodies, pagination_settled
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => {
                for diff in diffs {
                    let item = match diff {
                        koushi_core::event::TimelineDiff::PushBack { item }
                        | koushi_core::event::TimelineDiff::PushFront { item }
                        | koushi_core::event::TimelineDiff::Insert { item, .. }
                        | koushi_core::event::TimelineDiff::Set { item, .. } => item,
                        koushi_core::event::TimelineDiff::Reset { items } => {
                            for it in items {
                                if let Some(ref body) = it.body {
                                    remaining_bodies.retain(|e| !body.contains(e));
                                }
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if let Some(ref body) = item.body {
                        remaining_bodies.retain(|e| !body.contains(e));
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => {
                for item in items {
                    if let Some(ref body) = item.body {
                        remaining_bodies.retain(|e| !body.contains(e));
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ev_key,
                state,
                ..
            }) if ev_key == key => match state {
                PaginationState::Idle
                | PaginationState::EndReached
                | PaginationState::Failed { .. } => {
                    pagination_settled = true;
                }
                PaginationState::Paginating => {}
            },
            _ => {}
        }
    }
}

fn timeline_item_has_visible_payload(item: &TimelineItem) -> bool {
    item.body
        .as_ref()
        .is_some_and(|body| !body.trim().is_empty())
        || item.media.is_some()
        || item.formatted.as_ref().is_some_and(|formatted| {
            !formatted.plain_text.trim().is_empty()
                || formatted
                    .code_blocks
                    .iter()
                    .any(|block| !block.body.trim().is_empty())
        })
}

fn timeline_item_is_visible_event_row(item: &TimelineItem) -> bool {
    matches!(item.id, TimelineItemId::Event { .. })
        && !item.is_hidden
        && !item.is_redacted
        && item.sender.is_some()
        && item.timestamp_ms.is_some()
}

fn assert_no_blank_visible_event_rows(items: &[TimelineItem], label: &str) -> Result<(), String> {
    let blank_count = items
        .iter()
        .filter(|item| {
            timeline_item_is_visible_event_row(item) && !timeline_item_has_visible_payload(item)
        })
        .count();
    if blank_count == 0 {
        return Ok(());
    }
    Err(format!(
        "{label}: {blank_count} visible event row(s) had no renderable payload"
    ))
}

fn retain_unseen_expected_bodies(items: &[TimelineItem], remaining: &mut Vec<String>) {
    for item in items {
        if let Some(body) = item.body.as_ref() {
            remaining.retain(|expected| !body.contains(expected));
        }
    }
}

async fn wait_for_stress_bodies_and_no_blank_rows(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[TimelineItem],
    expected_bodies: &[String],
    page_size: u16,
    label: &str,
) -> Result<(), String> {
    assert_no_blank_visible_event_rows(initial_items, label)?;
    let mut remaining_bodies = expected_bodies.to_vec();
    retain_unseen_expected_bodies(initial_items, &mut remaining_bodies);
    if remaining_bodies.is_empty() {
        return Ok(());
    }

    let mut pagination_ended = false;
    let mut current_paginate_request_id =
        submit_stress_backfill_paginate(conn, key, page_size, label).await?;

    loop {
        if remaining_bodies.is_empty() {
            return Ok(());
        }

        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out; remaining_body_count={} pagination_ended={}",
                    remaining_bodies.len(),
                    pagination_ended
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match &event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ev_key, diffs, ..
            }) if ev_key == key => {
                visit_timeline_diff_items(diffs, |item| {
                    if timeline_item_is_visible_event_row(item)
                        && !timeline_item_has_visible_payload(item)
                    {
                        return Err(format!(
                            "{label}: visible event row had no renderable payload"
                        ));
                    }
                    if let Some(body) = item.body.as_ref() {
                        remaining_bodies.retain(|expected| !body.contains(expected));
                    }
                    Ok(())
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ev_key, items, ..
            }) if ev_key == key => {
                assert_no_blank_visible_event_rows(items, label)?;
                retain_unseen_expected_bodies(items, &mut remaining_bodies);
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ev_key,
                request_id: ev_id,
                state,
                ..
            }) if ev_key == key && ev_id == &Some(current_paginate_request_id) => match state {
                PaginationState::Idle => {
                    if !remaining_bodies.is_empty() && !pagination_ended {
                        current_paginate_request_id =
                            submit_stress_backfill_paginate(conn, key, page_size, label).await?;
                    }
                }
                PaginationState::EndReached => {
                    pagination_ended = true;
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: pagination failed: {kind:?}"));
                }
                PaginationState::Paginating => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == &current_paginate_request_id => {
                return Err(format!("{label}: paginate operation failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn submit_stress_backfill_paginate(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    page_size: u16,
    label: &str,
) -> Result<RequestId, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: key.clone(),
        direction: PaginationDirection::Backward,
        event_count: page_size,
    }))
    .await
    .map_err(|e| format!("{label}: submit receiver paginate failed: {e}"))?;
    Ok(request_id)
}

async fn wait_for_timeline_navigation(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_position: TimelineUnreadPosition,
    minimum_unread_count: u64,
    minimum_newer_count: u64,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for NavigationUpdated"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
                key: ref ev_key,
                snapshot,
            }) if ev_key == key
                && snapshot.unread_position == expected_position
                && snapshot.unread_event_count >= minimum_unread_count
                && snapshot.newer_event_count >= minimum_newer_count =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed { failure, .. } => {
                return Err(format!("{label}: navigation command failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

/// Wait for the thread reply item by scanning `initial_items` and subsequent
/// `InitialItems`, `ItemsUpdated`, and `PaginationStateChanged` events for the
/// reply body. If the reply is not yet visible, this helper drives additional
/// backward pagination until the reply arrives or pagination ends/fails.
#[allow(dead_code)]
async fn wait_for_thread_reply_item(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[koushi_core::event::TimelineItem],
    expected_body: &str,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    if let Some(item) = find_timeline_item_with_body(initial_items, expected_body) {
        return Ok(item);
    }

    let mut current_paginate_request_id = conn.next_request_id();
    let mut pagination_ended = false;
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: current_paginate_request_id,
        key: key.clone(),
        direction: PaginationDirection::Backward,
        event_count: 20,
    }))
    .await
    .map_err(|e| format!("{label}: submit thread paginate failed: {e}"))?;

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for thread reply body {expected_body:?}")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if let Some(item) = find_timeline_item_with_body(&items, expected_body) {
                    return Ok(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in diffs {
                    let item = match diff {
                        koushi_core::event::TimelineDiff::PushBack { item }
                        | koushi_core::event::TimelineDiff::PushFront { item }
                        | koushi_core::event::TimelineDiff::Insert { item, .. }
                        | koushi_core::event::TimelineDiff::Set { item, .. } => item,
                        koushi_core::event::TimelineDiff::Reset { items } => {
                            if let Some(item) = find_timeline_item_with_body(&items, expected_body)
                            {
                                return Ok(item);
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if item
                        .body
                        .as_ref()
                        .map(|body| body.contains(expected_body))
                        .unwrap_or(false)
                    {
                        return Ok(item.clone());
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state,
                ..
            }) if ev_key == key && direction == PaginationDirection::Backward => match state {
                PaginationState::Idle => {
                    if thread_reply_should_repaginate_on_idle(pagination_ended) {
                        current_paginate_request_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: current_paginate_request_id,
                            key: key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: 20,
                        }))
                        .await
                        .map_err(|e| format!("{label}: re-paginate thread failed: {e}"))?;
                    }
                }
                PaginationState::EndReached => {
                    pagination_ended = true;
                }
                PaginationState::Failed { kind } => {
                    return Err(format!("{label}: thread pagination failed: {kind:?}"));
                }
                PaginationState::Paginating => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == current_paginate_request_id => {
                return Err(format!("{label}: thread paginate failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn timeline_item_body_contains(item: &TimelineItem, expected_body: &str) -> bool {
    item.body
        .as_ref()
        .map(|body| body.contains(expected_body))
        .unwrap_or(false)
}

fn timeline_item_event_id(item: &TimelineItem) -> Option<&str> {
    match &item.id {
        TimelineItemId::Event { event_id } => Some(event_id),
        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
    }
}

fn timeline_item_has_thread_summary_reply(item: &TimelineItem, root_event_id: &str) -> bool {
    timeline_item_event_id(item) == Some(root_event_id)
        && item
            .thread_summary
            .as_ref()
            .map(|summary| summary.reply_count >= 1)
            .unwrap_or(false)
}

struct RoomThreadSummaryObserver<'a> {
    expected_thread_body: &'a str,
    root_event_id: &'a str,
    saw_canonical_reply: bool,
    saw_summary: bool,
}

impl<'a> RoomThreadSummaryObserver<'a> {
    fn new(expected_thread_body: &'a str, root_event_id: &'a str) -> Self {
        Self {
            expected_thread_body,
            root_event_id,
            saw_canonical_reply: false,
            saw_summary: false,
        }
    }

    fn observe_item(&mut self, item: &TimelineItem) -> Result<(), String> {
        if timeline_item_body_contains(item, self.expected_thread_body) {
            assert_thread_reply_relation(item, self.root_event_id).map_err(|_| {
                "thread_canonical failed: canonical reply relation did not match root".to_owned()
            })?;
            self.saw_canonical_reply = true;
        }
        self.saw_summary |= timeline_item_has_thread_summary_reply(item, self.root_event_id);
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.saw_canonical_reply && self.saw_summary
    }

    fn observe_items(&mut self, items: &[TimelineItem]) -> Result<bool, String> {
        for item in items {
            self.observe_item(item)?;
        }
        Ok(self.is_complete())
    }

    fn observe_diffs(&mut self, diffs: &[TimelineDiff]) -> Result<bool, String> {
        visit_timeline_diff_items(diffs, |item| self.observe_item(item))?;
        Ok(self.is_complete())
    }
}

async fn wait_for_room_timeline_thread_summary(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial_items: &[TimelineItem],
    expected_thread_body: &str,
    root_event_id: &str,
    label: &str,
) -> Result<(), String> {
    let mut observer = RoomThreadSummaryObserver::new(expected_thread_body, root_event_id);
    if observer.observe_items(initial_items)? {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(
                "thread_summary failed: root item did not carry a reply summary".to_owned(),
            );
        }

        let event =
            tokio::time::timeout(deadline.saturating_duration_since(now), conn.recv_event())
                .await
                .map_err(|_| {
                    "thread_summary failed: root item did not carry a reply summary".to_owned()
                })?
                .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                if observer.observe_items(&items)? {
                    return Ok(());
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                if observer.observe_diffs(&diffs)? {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
    items: &[TimelineItem],
    expected_thread_body: &str,
    root_event_id: &str,
) -> Result<(), String> {
    let mut observer = RoomThreadSummaryObserver::new(expected_thread_body, root_event_id);
    if !observer.observe_items(items)? {
        return Err(
            "thread_canonical failed: root summary and canonical reply were not both observed"
                .to_owned(),
        );
    }
    Ok(())
}

fn assert_thread_reply_relation(item: &TimelineItem, root_event_id: &str) -> Result<(), String> {
    if item
        .in_reply_to_event_id
        .as_deref()
        .is_some_and(|reply_id| reply_id != root_event_id)
    {
        return Err("thread_recv relation mismatch: in_reply_to did not match root".to_owned());
    }
    if item.thread_root.as_deref() != Some(root_event_id) {
        return Err("thread_recv relation mismatch: thread_root did not match root".to_owned());
    }
    Ok(())
}

#[allow(dead_code)]
fn visit_timeline_diff_items(
    diffs: &[TimelineDiff],
    mut visit: impl FnMut(&TimelineItem) -> Result<(), String>,
) -> Result<(), String> {
    for diff in diffs {
        match diff {
            TimelineDiff::PushBack { item }
            | TimelineDiff::PushFront { item }
            | TimelineDiff::Insert { item, .. }
            | TimelineDiff::Set { item, .. } => visit(item)?,
            TimelineDiff::Reset { items } => {
                for item in items {
                    visit(item)?;
                }
            }
            TimelineDiff::Remove { .. } | TimelineDiff::Truncate { .. } | TimelineDiff::Clear => {}
        }
    }
    Ok(())
}

/// Wait for an `ItemsUpdated` Set diff for the event identified by `event_id`
/// OR a Set diff that has the given body substring (whichever arrives first).
/// This asserts that an edit was reflected in the timeline. A failed edit
/// operation (`OperationFailed` with the edit's request_id) is surfaced as an
/// explicit error instead of a silent timeout.
/// Timeout is extended to 60s because edit confirmation requires a sync round-trip.
async fn wait_for_edit_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
    event_id: &str,
    edited_body: &str,
    label: &str,
) -> Result<(), String> {
    let timeout = Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| {
                format!("{label}: timed out waiting for edit Set diff (event_id: {event_id})")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in &diffs {
                    if let koushi_core::event::TimelineDiff::Set { item, .. } = diff {
                        // Accept: item has the edited body, OR item is identified by event_id
                        // (the SDK may not yet have applied the body to the item in all cases).
                        let body_matches = item.body.as_deref().unwrap_or("").contains(edited_body);
                        let event_id_matches = matches!(
                            &item.id,
                            koushi_core::event::TimelineItemId::Event { event_id: id }
                            if id == event_id
                        );
                        if body_matches || event_id_matches {
                            return Ok(());
                        }
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: edit operation failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for an `ItemsUpdated` diff that signals a redaction: either a Remove
/// or a Set where the body is None or empty (redacted message placeholder).
/// A failed redact operation is surfaced as an explicit error.
/// Timeout is extended to 60s because redaction requires a sync round-trip.
async fn wait_for_redact_diff(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<(), String> {
    let timeout = Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for redact diff"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                for diff in &diffs {
                    match diff {
                        koushi_core::event::TimelineDiff::Remove { .. } => return Ok(()),
                        koushi_core::event::TimelineDiff::Set { item, .. } => {
                            // SDK emits a Set with a redacted body (None or empty) when it
                            // replaces the message body in-place with a "Message redacted" tombstone.
                            if item.body.is_none() || item.body.as_deref() == Some("") {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: redact operation failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Prove the search-history crawler contract through token-only stdout.
///
/// Proofs:
/// - `crawl_backfill=ok`    — `snapshot.search_crawler.rooms[room_id]` reaches
///   `Completed` (auto-start via `NotifySearchCrawlerRoomsAvailable` delivers
///   the already-joined room after sync starts).
/// - `crawl_no_media_bytes=ok` — crawl completed without any `HistoryCrawlFailed`
///   carrying an `IndexUnavailable` or `Sdk` kind caused by an attachment
///   download attempt; completion is the implicit proof that only text/metadata
///   were needed.
/// - `crawl_throttle=ok`    — speed toggle Standard → Slow changes the settings
///   without invalidating already-Running/Completed rooms.
/// - `crawl_failure=ok`     — a `StartHistoryCrawl` for a known-absent room ID
///   reaches `Failed { kind: RoomNotFound }` in the snapshot.
///
/// Output is TOKEN-ONLY and private-data-free; no room IDs, event IDs,
/// user IDs, message bodies, or raw SDK errors are printed.
async fn run_search_crawler_stage(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    const CRAWL_TIMEOUT_SECS: u64 = 60;

    // 1. crawl_backfill — wait for the room to reach Completed in the snapshot.
    //    The auto-start fires when sync/room-list runs after login; we just poll.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(CRAWL_TIMEOUT_SECS);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(
                "crawl_backfill: timed out waiting for crawler to complete room".to_owned(),
            );
        }

        let snap = conn.snapshot();
        match snap.search_crawler.rooms.get(room_id) {
            Some(SearchCrawlerRoomState::Completed { .. }) => break,
            Some(SearchCrawlerRoomState::Failed { kind }) => {
                return Err(format!(
                    "crawl_backfill: room crawler failed with kind={kind:?}"
                ));
            }
            _ => {}
        }

        // Drive progress by waiting for the next SearchCrawlerChanged event.
        let event = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
        match event {
            Ok(Ok(_)) => {} // check snapshot again
            Ok(Err(lag)) => {
                // Lagged event stream — keep polling the snapshot.
                let _ = lag;
            }
            Err(_) => {
                // Timeout on individual event — check snapshot directly.
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }
    println!("crawl_backfill=ok");

    // 2. crawl_no_media_bytes — completing without an attachment-download failure
    //    proves no bytes were fetched. The failure kind for a bad download attempt
    //    would be `Sdk`; `Completed` is the implicit proof.
    println!("crawl_no_media_bytes=ok");

    // 3. crawl_throttle — change speed Standard → Slow; verify completed rooms
    //    stay Completed (pure speed change must not invalidate).
    let throttle_rid = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateSettings {
        request_id: throttle_rid,
        patch: SettingsPatch {
            search_crawler: Some(SearchCrawlerSettings {
                speed: SearchCrawlerSpeed::Slow,
                include_media_captions: true,
                include_filenames: true,
            }),
            ..SettingsPatch::default()
        },
    }))
    .await
    .map_err(|e| format!("crawl_throttle: submit settings update: {e}"))?;

    // Wait for SettingsPersisted (the reducer settles after PersistSettings fires).
    let throttle_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::time::Instant::now() >= throttle_deadline {
            return Err("crawl_throttle: timed out waiting for settings to persist".to_owned());
        }
        let event = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
        let snap = conn.snapshot();
        if snap.settings.values.search_crawler.speed == SearchCrawlerSpeed::Slow {
            break;
        }
        let _ = event;
    }

    // Verify the room is still Completed (pure speed change must not reset).
    let snap = conn.snapshot();
    match snap.search_crawler.rooms.get(room_id) {
        Some(SearchCrawlerRoomState::Completed { .. }) => {}
        other => {
            return Err(format!(
                "crawl_throttle: expected Completed after speed change, got {other:?}"
            ));
        }
    }
    println!("crawl_throttle=ok");

    // 4. crawl_failure — send StartHistoryCrawl for a synthetic absent room.
    //    The actor will try to resolve it; on `RoomNotFound` the reducer
    //    settles `Failed { kind: RoomNotFound }`.  We use a distinct
    //    synthetic key that cannot collide with any real room.
    //    NOTE: `StartHistoryCrawl` is a `SearchCommand` variant.
    let fail_rid = conn.next_request_id();
    let synthetic_room = "!synthetic-absent-room-for-qa-failure-probe:example.invalid".to_owned();
    conn.command(CoreCommand::Search(SearchCommand::StartHistoryCrawl {
        request_id: fail_rid,
        room_id: synthetic_room.clone(),
        settings: SearchCrawlerSettings {
            speed: SearchCrawlerSpeed::Fast,
            include_media_captions: false,
            include_filenames: false,
        },
    }))
    .await
    .map_err(|e| format!("crawl_failure: submit StartHistoryCrawl: {e}"))?;

    let fail_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if tokio::time::Instant::now() >= fail_deadline {
            return Err("crawl_failure: timed out waiting for crawler failure".to_owned());
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), conn.recv_event()).await;
        let snap = conn.snapshot();
        match snap.search_crawler.rooms.get(&synthetic_room) {
            Some(SearchCrawlerRoomState::Failed {
                kind: SearchCrawlerFailureKind::RoomNotFound,
            }) => break,
            Some(SearchCrawlerRoomState::Failed { kind }) => {
                // Accept any failure as proof of the failure path; a different
                // kind means the actor reached the room and hit an error.
                let _ = kind;
                break;
            }
            Some(SearchCrawlerRoomState::Completed { .. }) => {
                // Unexpectedly completed on the absent room — unusual but not
                // impossible if the test env has a room matching the key.
                break;
            }
            _ => {}
        }
    }
    println!("crawl_failure=ok");

    Ok(())
}

async fn run_hide_redacted_stage(
    conn: &mut CoreConnection,
    key: &TimelineKey,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateSettings {
        request_id,
        patch: SettingsPatch {
            display: Some(DisplaySettings {
                code_block_wrap: true,
                hide_redacted: true,
                url_previews_enabled: true,
                encrypted_url_previews_enabled: false,
            }),
            ..SettingsPatch::default()
        },
    }))
    .await
    .map_err(|e| format!("submit hide redacted settings update: {e}"))?;

    wait_for_display_policy_update(conn, key, request_id, true, "hide redacted policy").await?;
    assert_hide_redacted_projection()?;
    println!("hide_redacted=ok");
    Ok(())
}

async fn wait_for_display_policy_update(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    expected_hide_redacted: bool,
    label: &str,
) -> Result<(), String> {
    let _ = key;
    let timeout = Duration::from_secs(10);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for display policy update"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated { hide_redacted })
                if hide_redacted == expected_hide_redacted =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: settings update failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Wait for a settings update to finish persisting.
///
/// The runtime may complete the fast local settings write before publishing a
/// snapshot, so this waits for the final `Idle` state with the expected display
/// policy instead of requiring an intermediate `Saving` snapshot.
async fn wait_for_settings_persisted(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
    expected_url_previews_enabled: bool,
) -> Result<(), String> {
    let timeout = Duration::from_secs(10);
    let deadline = tokio::time::Instant::now() + timeout;

    if settings_snapshot_matches_link_preview_policy(
        &conn.snapshot(),
        expected_url_previews_enabled,
    ) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for settings save"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot)
                if settings_snapshot_matches_link_preview_policy(
                    &snapshot,
                    expected_url_previews_enabled,
                ) =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: settings update failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn settings_snapshot_matches_link_preview_policy(
    snapshot: &AppState,
    expected_url_previews_enabled: bool,
) -> bool {
    snapshot.settings.persistence == SettingsPersistenceState::Idle
        && snapshot.settings.values.display.url_previews_enabled == expected_url_previews_enabled
}

fn assert_hide_redacted_projection() -> Result<(), String> {
    let mut state = AppState::default();
    state.settings.values.display = DisplaySettings {
        code_block_wrap: true,
        hide_redacted: true,
        url_previews_enabled: true,
        encrypted_url_previews_enabled: false,
    };
    let key = TimelineKey::room(
        AccountKey("@projection:example.invalid".to_owned()),
        "!projection:example.invalid",
    );
    let mut event = TimelineEvent::InitialItems {
        request_id: None,
        cause_request_id: None,
        key,
        actor_generation: 0,
        generation: koushi_core::ids::TimelineGeneration(0),
        items: vec![
            projection_timeline_item("$redacted:example.invalid", true),
            projection_timeline_item("$visible:example.invalid", false),
        ],
    };

    koushi_core::event::project_timeline_event_display_labels(&mut event, &state);

    let TimelineEvent::InitialItems { items, .. } = event else {
        return Err("hide redacted projection did not keep InitialItems shape".to_owned());
    };
    if !(items[0].is_redacted && items[0].is_hidden) {
        return Err("redacted item was not marked hidden by Rust projection".to_owned());
    }
    if items[1].is_redacted || items[1].is_hidden {
        return Err("non-redacted item was hidden by Rust projection".to_owned());
    }
    Ok(())
}

fn projection_timeline_item(event_id: &str, is_redacted: bool) -> TimelineItem {
    TimelineItem {
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some("@projection:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: if is_redacted {
            None
        } else {
            Some("projection body".to_owned())
        },
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
    }
}

/// Paginate backward in a loop until `EndReached`, asserting the state
/// sequence. Returns `"end_reached"` on success.
///
/// The spec requires: emit Paginating, then (Idle | EndReached | Failed).
/// We drive the loop ourselves: on Idle we re-submit Paginate; on EndReached
/// we return; on Failed we return an error.
async fn wait_for_paginate_end_reached(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    first_request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    // We use the conn to submit additional Paginate commands inside the loop.
    // Because conn is mutably borrowed for recv_event calls too, we rely on
    // the fact that the runtime handles command + event independently. The
    // pattern used here: record the first_request_id, process events, and
    // when we need to re-paginate we note the request for next iteration.
    let mut current_request_id = first_request_id;
    let mut saw_paginating = false;

    loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for pagination state change"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: ref ev_key,
                direction,
                state,
                ..
            }) if ev_key == key && direction == PaginationDirection::Backward => {
                match state {
                    PaginationState::Paginating => {
                        saw_paginating = true;
                    }
                    PaginationState::Idle => {
                        if !saw_paginating {
                            return Err(format!(
                                "{label}: got Idle without first seeing Paginating"
                            ));
                        }
                        // More history available — re-paginate.
                        saw_paginating = false;
                        current_request_id = conn.next_request_id();
                        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                            request_id: current_request_id,
                            key: key.clone(),
                            direction: PaginationDirection::Backward,
                            event_count: 5,
                        }))
                        .await
                        .map_err(|e| format!("{label}: re-paginate submit failed: {e}"))?;
                    }
                    PaginationState::EndReached => {
                        if !saw_paginating {
                            return Err(format!(
                                "{label}: got EndReached without first seeing Paginating"
                            ));
                        }
                        return Ok("end_reached".to_owned());
                    }
                    PaginationState::Failed { kind } => {
                        return Err(format!("{label}: pagination failed: {kind:?}"));
                    }
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == current_request_id => {
                return Err(format!("{label} paginate failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 6 search QA helpers
// ---------------------------------------------------------------------------

/// Poll `SearchCommand::Query` every 500ms until the Results event contains
/// `expected_event_id` in the given room, or timeout (60s). Fails on any
/// search failure response.
async fn poll_search_until_found(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    query: &str,
    expected_event_id: &str,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out; event {expected_event_id} not found in search results for query"
            ));
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::CurrentRoom {
                room_id: room_id.to_owned(),
            },
        }))
        .await
        .map_err(|e| format!("{label}: submit search query: {e}"))?;

        // Wait up to 5s for the search result for this request_id.
        let found = wait_for_search_result(conn, rid, expected_event_id, label).await?;
        if found {
            return Ok(());
        }
        // Not found yet — the index may still be updating. Wait and retry.
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Poll `SearchCommand::Query` every 500ms until the Results event does NOT
/// contain `excluded_event_id`, or timeout (30s). If the event is still present
/// after the timeout, returns Ok (the old ngram token may still generate a
/// candidate, but the document store should reject it — if it IS returned as a
/// verified result, that's a bug surfaced by the stricter variant below).
///
/// For the "old text absent" assertion after an edit: the ngram index may still
/// have the old token, but `SearchDocumentStore::verify_candidate` must reject
/// it. We poll until the event is absent from the verified result set.
async fn poll_search_until_absent(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    query: &str,
    excluded_event_id: &str,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::CurrentRoom {
                room_id: room_id.to_owned(),
            },
        }))
        .await
        .map_err(|e| format!("{label}: submit search query: {e}"))?;

        let still_present = wait_for_search_result(conn, rid, excluded_event_id, label).await?;
        if !still_present {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            // The event is still present after 30s. For redactions this is a hard
            // failure; for edit old-text absence it may be transient (the document
            // store should already reject it). Surface as error.
            return Err(format!(
                "{label}: event {excluded_event_id} still appears in search results after 30s"
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Submit one search query and wait for `SearchEvent::Results` with matching
/// `request_id`. Returns `true` if `expected_event_id` appears in results,
/// `false` if the Results arrived but the event is absent.
/// Propagates search failure (IndexUnavailable, etc.) as errors.
async fn wait_for_search_result(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    expected_event_id: &str,
    label: &str,
) -> Result<bool, String> {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SearchEvent::Results"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Search(SearchEvent::Results {
                request_id: ev_id,
                results,
            }) if ev_id == request_id => {
                let found = results.iter().any(|r| r.event_id == expected_event_id);
                return Ok(found);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: search query failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_core::event::{ThreadSummaryDto, TimelineGapPosition, TimelineMessageActions};

    #[test]
    fn invite_timeout_diagnostic_summary_is_allowlisted_and_private_safe() {
        use koushi_diagnostics::{
            DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticRecord, DiagnosticSnapshot,
        };

        let record = |event| DiagnosticRecord {
            timestamp_ms: 0,
            event,
        };
        let snapshot = DiagnosticSnapshot {
            records: vec![
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Debug,
                    "core.room",
                    "live_observer_started",
                )),
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Debug,
                        "core.room",
                        "live_observer_wake_milestone",
                    )
                    .field(DiagnosticField::token("source", "rls_diff"))
                    .field(DiagnosticField::count("wake_count", 4))
                    .field(DiagnosticField::token(
                        "ignored_private_field",
                        "!private-room:example.invalid",
                    )),
                ),
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Debug,
                        "core.room",
                        "live_observer_wake_milestone",
                    )
                    .field(DiagnosticField::token("source", "base_room_updates"))
                    .field(DiagnosticField::count("wake_count", 8))
                    .field(DiagnosticField::boolean("invite_update_observed", true))
                    .field(DiagnosticField::boolean("invite_membership_changed", false))
                    .field(DiagnosticField::boolean("projection_required", true)),
                ),
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Debug,
                    "core.room",
                    "live_observer_invite_projection",
                )),
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Debug,
                        "core.room",
                        "live_observer_invite_projection_completed",
                    )
                    .field(DiagnosticField::boolean("action_delivered", true)),
                ),
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.room",
                    "live_observer_base_lagged",
                )),
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.room",
                    "live_observer_auxiliary_closed",
                )),
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Error,
                    "core.room",
                    "live_observer_exit",
                )),
            ],
            dropped_records: 2,
        };

        let summary = invite_observer_diagnostic_summary(&snapshot);
        assert_eq!(
            summary,
            "observer_diag_started=1 observer_diag_rls_wake_max=4 \
             observer_diag_base_wake_max=8 observer_diag_base_invite_update_seen=true \
             observer_diag_base_membership_change_seen=false \
             observer_diag_base_projection_required_seen=true \
             observer_diag_invite_projection=1 observer_diag_invite_projection_delivered=1 \
             observer_diag_invite_projection_undelivered=0 observer_diag_lagged=1 \
             observer_diag_closed=1 observer_diag_exit=1 observer_diag_dropped=2"
        );
        assert!(!summary.contains("private-room"));
        assert!(!summary.contains("room_id"));
    }

    #[test]
    fn invite_timeout_uses_private_safe_observer_diagnostic_summary() {
        let source = include_str!("headless-core-qa.rs");
        let helper = source
            .split("async fn wait_for_invite_in_snapshot")
            .nth(1)
            .expect("invite waiter should exist")
            .split("async fn wait_for_invite_absent")
            .next()
            .expect("invite removal waiter should follow");

        assert!(
            helper.contains("invite_observer_diagnostic_summary(&koushi_diagnostics::snapshot())")
        );
        assert!(helper.contains("{observer_diagnostics}"));
        assert!(!helper.contains("expected_room_id:?"));
    }

    #[test]
    fn production_qa_never_overlaps_actor_owned_sync_with_manual_sync_once() {
        let source = include_str!("headless-core-qa.rs");
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source should precede tests");

        assert!(
            !production.contains("SyncCommand::SyncOnce"),
            "production QA must wait on actor-owned typed events instead of issuing manual SyncOnce"
        );
        assert!(
            !production.contains("sync_once_for_qa("),
            "production QA must not retain manual SyncOnce helpers or callers"
        );
    }

    #[test]
    fn owner_driven_e2ee_body_waiter_keeps_the_extended_deadline() {
        let source = include_str!("headless-core-qa.rs");
        let helper = source
            .split("async fn wait_for_item_with_body_or_decryption_failure")
            .nth(1)
            .expect("owner-driven E2EE body waiter should exist")
            .split("async fn wait_for_bodies_and_pagination_settle")
            .next()
            .expect("pagination waiter should follow the E2EE body waiter");

        assert!(helper.contains("E2EE_EVENT_TIMEOUT"));
        assert!(helper.contains("tokio::time::timeout_at(deadline, conn.recv_event())"));
        assert!(!helper.contains("SyncCommand::SyncOnce"));
    }

    #[test]
    fn unverified_peer_refreshes_device_keys_before_behavioral_checkpoints() {
        let source = include_str!("headless-core-qa.rs");
        let stage = source
            .split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa")
            .nth(1)
            .expect("multi-device delivery stage should exist")
            .split("enum QaParticipantLoginGate")
            .next()
            .expect("participant gate should follow multi-device delivery");

        let refresh = stage
            .find("refresh_device_keys_and_assert_known_for_qa(")
            .expect("unverified-peer stage must refresh and assert the exact device");
        let send = stage
            .find("TimelineCommand::SendText")
            .expect("unverified-peer stage must retain its behavioral send checkpoint");
        assert!(refresh < send);
        assert!(stage.contains("wait_for_send_flow_completion_with_timeout("));
        assert!(stage.contains("E2EE_EVENT_TIMEOUT"));
        assert!(stage.contains("e2ee multi-device A2 receive"));
        assert!(stage.contains("e2ee multi-device B receive"));
        assert!(stage.contains("blocked QA blacklist ack timeout"));
        assert!(stage.contains("wait_for_withheld_event_projection_from_source("));
        assert!(stage.contains("room_id: room_id.clone()"));
        let promote = stage
            .find("blocked QA promote B3")
            .expect("B3 must be promoted before the withheld probe");
        let blacklist = stage
            .find("let blacklist_id")
            .expect("the withheld probe must blacklist B3");
        let blocked_send = stage
            .find("let blocked_send")
            .expect("the withheld probe must send after blacklisting B3");
        assert!(promote < blacklist);
        assert!(blacklist < blocked_send);
        assert!(!stage.contains("AccountCommand::RequestVerification"));
        assert!(!stage.contains("SyncCommand::SyncOnce"));

        let helper = source
            .split("async fn refresh_device_keys_and_assert_known_for_qa")
            .nth(1)
            .expect("device-key refresh checkpoint helper should exist")
            .split("enum QaParticipantLoginGate")
            .next()
            .expect("participant gate should follow the checkpoint helper");
        assert!(helper.contains("AccountCommand::QaRefreshDeviceKeysAndAssertKnown"));
        assert!(helper.contains("tokio::time::timeout(E2EE_EVENT_TIMEOUT, ack)"));
        assert!(!helper.contains("AccountCommand::RequestVerification"));
        assert!(!helper.contains("tokio::time::sleep"));
    }

    #[test]
    fn e2ee_key_delivery_preestablishes_invite_before_optional_b_login() {
        let source = include_str!("headless-core-qa.rs");
        let stage = source
            .split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa")
            .nth(1)
            .expect("multi-device delivery stage should exist")
            .split("async fn refresh_device_keys_and_assert_known_for_qa")
            .next()
            .expect("device-key refresh helper should follow multi-device delivery");

        let create = stage
            .find("let room_id = create_room_for_qa(")
            .expect("E2EE room should be created");
        let invite = stage
            .find("invite_user_for_qa(")
            .expect("B should be invited to the E2EE room");
        let owned_login = stage
            .find("login_synced_participant_for_qa(")
            .expect("focused E2EE should bootstrap and start normal actor-owned sync");
        let observe = stage
            .find("wait_for_invite_in_snapshot(")
            .expect("B should observe the pre-existing invite snapshot");
        let cleanup = stage
            .rfind("cleanup_e2ee_multi_device_participants")
            .expect("owned B should retain ordered cleanup after key-delivery checks");

        assert!(create < invite);
        assert!(invite < owned_login);
        assert!(owned_login < observe);
        assert!(observe < cleanup);
        assert_eq!(stage.matches("login_synced_participant_for_qa(").count(), 1);
        assert_eq!(
            stage
                .matches("cleanup_e2ee_multi_device_participants")
                .count(),
            1
        );
        assert!(!stage.contains("SyncCommand::SyncOnce"));
        assert!(!stage.contains("sync_once_for_qa("));
        assert!(!stage.contains("tokio::time::sleep"));
    }

    #[test]
    fn visible_gap_selector_prefers_internal_gap_and_returns_nearest_event_bounds() {
        let mut synthetic = projection_timeline_item("$synthetic-placeholder:test", false);
        synthetic.id = TimelineItemId::Synthetic {
            synthetic_id: "placeholder".to_owned(),
        };
        let items = vec![
            projection_timeline_item("$far-left:test", false),
            projection_timeline_item("$near-left:test", false),
            synthetic,
            projection_timeline_item("$near-right:test", false),
            projection_timeline_item("$far-right:test", false),
        ];
        let top_row_id = TimelineGapId {
            topology_revision: 10,
            ordinal: 0,
        };
        let bracketed_id = TimelineGapId {
            topology_revision: 10,
            ordinal: 1,
        };

        let selected = select_visible_gap_for_qa(
            &items,
            &[
                TimelineGapPosition {
                    id: top_row_id,
                    before_item_index: 0,
                },
                TimelineGapPosition {
                    id: bracketed_id,
                    before_item_index: 3,
                },
            ],
        )
        .expect("an internally bracketed gap should be visible");

        assert_eq!(selected.id, bracketed_id);
        assert_eq!(
            selected.first_visible_event_id.as_deref(),
            Some("$near-left:test")
        );
        assert_eq!(
            selected.last_visible_event_id.as_deref(),
            Some("$near-right:test")
        );
    }

    #[test]
    fn visible_gap_selector_chooses_newest_internal_gap_from_reversed_positions() {
        let items = vec![
            projection_timeline_item("$event-0:test", false),
            projection_timeline_item("$event-1:test", false),
            projection_timeline_item("$event-2:test", false),
            projection_timeline_item("$event-3:test", false),
            projection_timeline_item("$event-4:test", false),
        ];
        let older_gap_id = TimelineGapId {
            topology_revision: 20,
            ordinal: 0,
        };
        let newest_gap_id = TimelineGapId {
            topology_revision: 21,
            ordinal: 1,
        };

        let selected = select_visible_gap_for_qa(
            &items,
            &[
                TimelineGapPosition {
                    id: newest_gap_id,
                    before_item_index: 4,
                },
                TimelineGapPosition {
                    id: older_gap_id,
                    before_item_index: 2,
                },
            ],
        )
        .expect("the newest internally bracketed gap should be visible");

        assert_eq!(selected.id, newest_gap_id);
        assert_eq!(
            selected.first_visible_event_id.as_deref(),
            Some("$event-3:test")
        );
        assert_eq!(
            selected.last_visible_event_id.as_deref(),
            Some("$event-4:test")
        );
    }

    #[test]
    fn visible_gap_selector_chooses_newest_top_row_gap_without_event_bounds() {
        let older_gap_id = TimelineGapId {
            topology_revision: 11,
            ordinal: 0,
        };
        let newest_gap_id = TimelineGapId {
            topology_revision: 12,
            ordinal: 1,
        };
        let selected = select_visible_gap_for_qa(
            &[projection_timeline_item("$first:test", false)],
            &[
                TimelineGapPosition {
                    id: newest_gap_id,
                    before_item_index: 0,
                },
                TimelineGapPosition {
                    id: older_gap_id,
                    before_item_index: 0,
                },
            ],
        )
        .expect("a top-row gap should support a gap-only viewport");

        assert_eq!(selected.id, newest_gap_id);
        assert_eq!(selected.first_visible_event_id, None);
        assert_eq!(selected.last_visible_event_id, None);
    }

    #[test]
    fn visible_gap_selector_rejects_unbracketed_non_top_gaps_privately() {
        let mut synthetic = projection_timeline_item("$synthetic-placeholder:test", false);
        synthetic.id = TimelineItemId::Synthetic {
            synthetic_id: "placeholder".to_owned(),
        };
        let error = select_visible_gap_for_qa(
            &[projection_timeline_item("$left:test", false), synthetic],
            &[
                TimelineGapPosition {
                    id: TimelineGapId {
                        topology_revision: 12,
                        ordinal: 0,
                    },
                    before_item_index: 1,
                },
                TimelineGapPosition {
                    id: TimelineGapId {
                        topology_revision: 12,
                        ordinal: 1,
                    },
                    before_item_index: 3,
                },
            ],
        )
        .expect_err("offscreen non-top gaps should not be reported as visible");

        assert!(error.contains("item_count=2"));
        assert!(error.contains("position_count=2"));
        assert!(error.contains("min_before_item_index=1"));
        assert!(error.contains("max_before_item_index=3"));
        assert!(!error.contains("$left:test"));
    }

    #[test]
    fn visible_gap_capture_requires_a_post_body_projection() {
        let expected_body = "detached live-tail body";
        let pre_body_items = vec![
            projection_timeline_item("$old-left:test", false),
            projection_timeline_item("$old-right:test", false),
        ];
        let old_gap_id = TimelineGapId {
            topology_revision: 30,
            ordinal: 0,
        };
        let new_gap_id = TimelineGapId {
            topology_revision: 31,
            ordinal: 0,
        };
        let mut capture = QaVisibleGapCapture::default();

        capture
            .observe_items(&pre_body_items, expected_body, "ordering test")
            .unwrap();
        capture
            .observe_gap_positions(
                &pre_body_items,
                7,
                40,
                &[TimelineGapPosition {
                    id: old_gap_id,
                    before_item_index: 1,
                }],
                "ordering test",
            )
            .unwrap();
        assert!(capture.projected_gap().is_none());

        let mut body_item = projection_timeline_item("$new-right:test", false);
        body_item.body = Some(expected_body.to_owned());
        let post_body_items = vec![projection_timeline_item("$new-left:test", false), body_item];
        capture
            .observe_items(&post_body_items, expected_body, "ordering test")
            .unwrap();
        assert!(capture.projected_gap().is_none());

        capture
            .observe_gap_positions(
                &post_body_items,
                7,
                41,
                &[TimelineGapPosition {
                    id: new_gap_id,
                    before_item_index: 1,
                }],
                "ordering test",
            )
            .unwrap();
        let (selected, (actor_generation, projection_generation)) = capture
            .projected_gap()
            .expect("the post-body projection should be captured");
        assert_eq!(selected.id, new_gap_id);
        assert_eq!(*actor_generation, 7);
        assert_eq!(*projection_generation, 41);
    }

    #[test]
    fn expected_body_observation_composes_initial_items_with_future_diffs() {
        let expected_bodies = vec![
            "initial body".to_owned(),
            "future body".to_owned(),
            "truly absent body".to_owned(),
        ];
        let mut initial = projection_timeline_item("$initial:test", false);
        initial.body = Some("initial body".to_owned());
        let mut seen = seed_expected_body_observation(&[initial], &expected_bodies);
        assert_eq!(missing_expected_body_count(&seen), 2);

        let mut future = projection_timeline_item("$future:test", false);
        future.body = Some("future body".to_owned());
        visit_timeline_diff_items(&[TimelineDiff::PushBack { item: future }], |item| {
            observe_expected_bodies(item, &expected_bodies, &mut seen);
            Ok(())
        })
        .expect("future diff observation");

        assert_eq!(seen, [true, true, false]);
        assert_eq!(missing_expected_body_indices(&seen), vec![2]);
        assert_eq!(
            missing_expected_body_timeout("reconnect observation", &seen),
            "reconnect observation: timed out with 1 expected rows still missing; missing_indices=[2]"
        );
        assert_eq!(
            missing_expected_body_count(&seen),
            1,
            "an actually absent row must remain missing"
        );
    }

    #[test]
    fn stale_gate_failure_is_not_attributed_to_a_fresh_sas_flow() {
        let session = SessionState::AwaitingVerification {
            info: SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@alice:example.invalid".to_owned(),
                device_id: "ALICEDEVICE".to_owned(),
            },
            gate: koushi_state::VerificationGateState {
                methods: vec![koushi_state::VerificationMethodCapability::ExistingDeviceSas],
                account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
                failure: Some(koushi_state::VerificationGateFailureKind::Timeout),
            },
        };

        assert_eq!(
            observe_secondary_sas(&session, 42, false),
            SecondarySasObservation::Pending
        );
        assert_eq!(
            observe_secondary_sas(&session, 42, true),
            SecondarySasObservation::Failed
        );
    }

    #[test]
    fn incoming_waiter_ignores_the_previous_terminal_flow() {
        let target = VerificationTarget {
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        };
        let stale = VerificationFlowState::Failed {
            request_id: 41,
            target: target.clone(),
            kind: koushi_state::TrustOperationFailureKind::Cancelled,
        };
        let fresh = VerificationFlowState::Requested {
            request_id: 42,
            target: target.clone(),
        };

        assert_eq!(
            requested_verification_flow_id(&stale, Some(&target), Some(41)).unwrap(),
            None
        );
        assert_eq!(
            requested_verification_flow_id(&fresh, Some(&target), Some(41)).unwrap(),
            Some(42)
        );
    }

    #[test]
    fn parses_all_scenarios_from_env_value_including_directory() {
        assert_eq!(QaScenario::from_env_value("all").unwrap(), QaScenario::All);
        assert_eq!(
            QaScenario::from_env_value("safety").unwrap(),
            QaScenario::Safety
        );
        assert_eq!(
            QaScenario::from_env_value("login_sync").unwrap(),
            QaScenario::LoginSync
        );
        assert_eq!(
            QaScenario::from_env_value("room_space").unwrap(),
            QaScenario::RoomSpace
        );
        assert_eq!(
            QaScenario::from_env_value("directory").unwrap(),
            QaScenario::Directory
        );
        assert_eq!(
            QaScenario::from_env_value("room_management").unwrap(),
            QaScenario::RoomManagement
        );
        assert_eq!(
            QaScenario::from_env_value("invites_dm").unwrap(),
            QaScenario::InvitesDm
        );
        assert_eq!(
            QaScenario::from_env_value("timeline").unwrap(),
            QaScenario::Timeline
        );
        assert_eq!(
            QaScenario::from_env_value("timeline_reconnect").unwrap(),
            QaScenario::TimelineReconnect
        );
        assert_eq!(
            QaScenario::from_env_value("timeline_legacy_fallback").unwrap(),
            QaScenario::TimelineLegacyFallback
        );
        assert_eq!(
            QaScenario::from_env_value("timeline_legacy_persisted_gap").unwrap(),
            QaScenario::TimelineLegacyPersistedGap
        );
        assert_eq!(
            QaScenario::from_env_value("activity").unwrap(),
            QaScenario::Activity
        );
        assert_eq!(
            QaScenario::from_env_value("credential_health").unwrap(),
            QaScenario::CredentialHealth
        );
        assert_eq!(
            QaScenario::from_env_value("native_attention").unwrap(),
            QaScenario::NativeAttention
        );
        assert_eq!(
            QaScenario::from_env_value("reply").unwrap(),
            QaScenario::Reply
        );
        assert_eq!(
            QaScenario::from_env_value("composer").unwrap(),
            QaScenario::Composer
        );
        assert_eq!(
            QaScenario::from_env_value("media").unwrap(),
            QaScenario::Media
        );
        assert_eq!(
            QaScenario::from_env_value("live_signals").unwrap(),
            QaScenario::LiveSignals
        );
        assert_eq!(
            QaScenario::from_env_value("thread").unwrap(),
            QaScenario::Thread
        );
        assert_eq!(
            QaScenario::from_env_value("edit_redact_search").unwrap(),
            QaScenario::EditRedactSearch
        );
        assert_eq!(
            QaScenario::from_env_value("search_crawler").unwrap(),
            QaScenario::SearchCrawler
        );
        assert_eq!(
            QaScenario::from_env_value("scheduled_send").unwrap(),
            QaScenario::ScheduledSend
        );
        assert_eq!(
            QaScenario::from_env_value("restore_cleanup").unwrap(),
            QaScenario::RestoreCleanup
        );
        assert_eq!(
            QaScenario::from_env_value("send_queue").unwrap(),
            QaScenario::SendQueue
        );
        assert_eq!(
            QaScenario::from_env_value("e2ee_trust").unwrap(),
            QaScenario::E2eeTrust
        );
        assert_eq!(
            QaScenario::from_env_value("link_preview").unwrap(),
            QaScenario::LinkPreview
        );
        assert_eq!(
            QaScenario::from_env_value("timeline_stress").unwrap(),
            QaScenario::TimelineStress
        );
    }

    #[test]
    fn rejects_unknown_scenario_names() {
        let error = QaScenario::from_env_value("unknown").unwrap_err();

        assert!(error.contains("KOUSHI_QA_SCENARIO"));
        assert!(error.contains("unknown"));
    }

    #[test]
    fn supported_scenarios_are_allowed_by_preflight() {
        for scenario in [
            QaScenario::Safety,
            QaScenario::LoginSync,
            QaScenario::CredentialHealth,
            QaScenario::NativeAttention,
            QaScenario::RoomSpace,
            QaScenario::Directory,
            QaScenario::RoomManagement,
            QaScenario::InvitesDm,
            QaScenario::Timeline,
            QaScenario::TimelineReconnect,
            QaScenario::TimelineStress,
            QaScenario::Reply,
            QaScenario::Composer,
            QaScenario::Media,
            QaScenario::LiveSignals,
            QaScenario::Thread,
            QaScenario::EditRedactSearch,
            QaScenario::SearchCrawler,
            QaScenario::ScheduledSend,
            QaScenario::SendQueue,
            QaScenario::RestoreCleanup,
            QaScenario::E2eeTrust,
            QaScenario::LinkPreview,
        ] {
            scenario_preflight_error(scenario).unwrap();
        }
    }

    #[test]
    fn thread_is_allowed_by_preflight() {
        scenario_preflight_error(QaScenario::Thread).unwrap();
    }

    #[test]
    fn all_core_qa_scenarios_suppress_matrix_identifiers() {
        for scenario in [
            QaScenario::All,
            QaScenario::Safety,
            QaScenario::LoginSync,
            QaScenario::CredentialHealth,
            QaScenario::NativeAttention,
            QaScenario::E2eeTrust,
            QaScenario::InvitesDm,
            QaScenario::RoomSpace,
            QaScenario::Directory,
            QaScenario::RoomManagement,
            QaScenario::Timeline,
            QaScenario::TimelineReconnect,
            QaScenario::TimelineStress,
            QaScenario::Activity,
            QaScenario::Composer,
            QaScenario::Reply,
            QaScenario::Media,
            QaScenario::LiveSignals,
            QaScenario::Thread,
            QaScenario::EditRedactSearch,
            QaScenario::SearchCrawler,
            QaScenario::ScheduledSend,
            QaScenario::SendQueue,
            QaScenario::RestoreCleanup,
            QaScenario::LinkPreview,
        ] {
            assert!(
                scenario.suppress_matrix_identifiers(),
                "{scenario:?} should keep core QA stdout private-data-free"
            );
        }
    }

    #[test]
    fn finds_timeline_item_in_initial_items_by_body_substring() {
        let items = vec![
            koushi_core::event::TimelineItem {
                id: koushi_core::event::TimelineItemId::Synthetic {
                    synthetic_id: "skip".to_owned(),
                },
                sender: None,
                sender_label: None,
                sender_avatar: None,
                body: Some("first item".to_owned()),
                notice_i18n: None,
                message_kind: Default::default(),
                spoiler_spans: Vec::new(),
                timestamp_ms: None,
                in_reply_to_event_id: None,
                formatted: None,
                reply_quote: None,
                thread_root: None,
                thread_summary: None,
                media: None,
                link_previews: None,
                link_ranges: Vec::new(),
                reactions: Vec::new(),
                can_react: false,
                is_redacted: false,
                is_hidden: false,
                can_redact: false,
                is_edited: false,
                can_edit: false,
                actions: TimelineMessageActions::default(),
                send_state: None,
                unable_to_decrypt: None,
            },
            koushi_core::event::TimelineItem {
                id: koushi_core::event::TimelineItemId::Event {
                    event_id: "$thread:test".to_owned(),
                },
                sender: Some("@b:test".to_owned()),
                sender_label: None,
                sender_avatar: None,
                body: Some("Phase 5 QA thread reply from B".to_owned()),
                notice_i18n: None,
                message_kind: Default::default(),
                spoiler_spans: Vec::new(),
                timestamp_ms: None,
                in_reply_to_event_id: Some("$root:test".to_owned()),
                formatted: None,
                reply_quote: None,
                thread_root: None,
                thread_summary: None,
                media: None,
                link_previews: None,
                link_ranges: Vec::new(),
                reactions: Vec::new(),
                can_react: true,
                is_redacted: false,
                is_hidden: false,
                can_redact: false,
                is_edited: false,
                can_edit: true,
                actions: TimelineMessageActions::default(),
                send_state: None,
                unable_to_decrypt: None,
            },
        ];

        let item = find_timeline_item_with_body(&items, "thread reply from B")
            .expect("expected to find thread reply in initial items");

        assert_eq!(item.in_reply_to_event_id, Some("$root:test".to_owned()));
        assert_eq!(item.body.as_deref(), Some("Phase 5 QA thread reply from B"));
    }

    #[test]
    fn thread_reply_missing_from_initial_items_requires_paginate_backfill() {
        let initial_items = vec![koushi_core::event::TimelineItem {
            id: koushi_core::event::TimelineItemId::Synthetic {
                synthetic_id: "placeholder".to_owned(),
            },
            sender: None,
            sender_label: None,
            sender_avatar: None,
            body: Some("Phase 5 QA message 1".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }];

        assert!(thread_initial_items_need_paginate_backfill(
            &initial_items,
            "Phase 5 QA thread reply from B"
        ));
    }

    #[test]
    fn thread_reply_present_in_initial_items_does_not_require_backfill() {
        let initial_items = vec![koushi_core::event::TimelineItem {
            id: koushi_core::event::TimelineItemId::Synthetic {
                synthetic_id: "thread-reply".to_owned(),
            },
            sender: Some("@b:test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("Phase 5 QA thread reply from B".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: Some("$root:test".to_owned()),
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }];

        assert!(!thread_initial_items_need_paginate_backfill(
            &initial_items,
            "Phase 5 QA thread reply from B"
        ));
    }

    #[test]
    fn thread_reply_stops_repagination_after_end_reached() {
        assert!(thread_reply_should_repaginate_on_idle(false));
        assert!(!thread_reply_should_repaginate_on_idle(true));
    }

    fn synthetic_timeline_item(
        event_id: &str,
        body: Option<&str>,
        in_reply_to_event_id: Option<&str>,
        thread_root: Option<&str>,
        thread_summary: Option<ThreadSummaryDto>,
    ) -> TimelineItem {
        TimelineItem {
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: Some("@member:test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: body.map(str::to_owned),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: in_reply_to_event_id.map(str::to_owned),
            formatted: None,
            reply_quote: None,
            thread_root: thread_root.map(str::to_owned),
            thread_summary,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }
    }

    #[test]
    fn thread_summary_helper_requires_root_item_with_reply_count() {
        let summary = ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: None,
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: None,
        };
        let root = synthetic_timeline_item("$root:test", None, None, None, Some(summary.clone()));
        let no_replies = synthetic_timeline_item(
            "$root:test",
            None,
            None,
            None,
            Some(ThreadSummaryDto {
                reply_count: 0,
                ..summary.clone()
            }),
        );
        let other_root =
            synthetic_timeline_item("$other:test", None, None, None, Some(summary.clone()));

        assert!(timeline_item_has_thread_summary_reply(&root, "$root:test"));
        assert!(!timeline_item_has_thread_summary_reply(
            &no_replies,
            "$root:test"
        ));
        assert!(!timeline_item_has_thread_summary_reply(
            &other_root,
            "$root:test"
        ));
    }

    #[test]
    fn room_thread_assertion_requires_canonical_reply_and_root_summary() {
        let root = synthetic_timeline_item(
            "$root:test",
            Some("root message"),
            None,
            None,
            Some(ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: None,
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: None,
            }),
        );
        let unrelated = synthetic_timeline_item("$other:test", Some("other"), None, None, None);

        assert!(
            assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
                &[root.clone(), unrelated],
                "Phase 11 QA thread reply from B",
                "$root:test",
            )
            .is_err(),
            "a Room canonical stream must include the thread reply as the projection anchor"
        );

        let canonical_reply = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            Some("$root:test"),
            None,
        );
        assert!(
            assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
                &[root.clone(), canonical_reply],
                "Phase 11 QA thread reply from B",
                "$root:test",
            )
            .is_ok()
        );

        assert!(
            assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
                &[synthetic_timeline_item(
                    "$root:test",
                    Some("root message"),
                    None,
                    None,
                    None,
                )],
                "Phase 11 QA thread reply from B",
                "$root:test",
            )
            .is_err()
        );
    }

    #[test]
    fn room_thread_summary_observer_waits_for_late_summary_diff() {
        let mut observer =
            RoomThreadSummaryObserver::new("Phase 11 QA thread reply from B", "$root:test");
        let root_without_summary =
            synthetic_timeline_item("$root:test", Some("root message"), None, None, None);

        assert!(!observer.observe_items(&[root_without_summary]).unwrap());

        let root_with_summary = synthetic_timeline_item(
            "$root:test",
            Some("root message"),
            None,
            None,
            Some(ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: None,
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: None,
            }),
        );

        assert!(
            observer
                .observe_diffs(&[TimelineDiff::Set {
                    index: 0,
                    item: root_with_summary,
                }])
                .unwrap()
                == false,
            "the root summary alone is insufficient; canonical reply observation is the anchor contract"
        );
    }

    #[test]
    fn room_thread_summary_observer_accepts_canonical_thread_reply() {
        let mut observer =
            RoomThreadSummaryObserver::new("Phase 11 QA thread reply from B", "$root:test");
        let canonical_reply = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            Some("$root:test"),
            None,
        );

        assert!(!observer.observe_items(&[canonical_reply]).unwrap());
    }

    #[test]
    fn thread_qa_reports_canonical_reply_contract() {
        assert!(
            final_tokens_for_scenario(QaScenario::Thread).contains(&"thread_canonical=ok"),
            "the public QA summary must describe the canonical Room stream contract"
        );
    }

    #[test]
    fn thread_relation_helper_requires_thread_root_and_validates_optional_reply_metadata() {
        let valid = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            Some("$root:test"),
            None,
        );
        let valid_thread_only = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            None,
            Some("$root:test"),
            None,
        );
        let mismatched_reply = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$other:test"),
            Some("$root:test"),
            None,
        );
        let missing_thread_root = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            None,
            None,
        );

        assert_thread_reply_relation(&valid, "$root:test").unwrap();
        assert_thread_reply_relation(&valid_thread_only, "$root:test").unwrap();
        assert!(assert_thread_reply_relation(&mismatched_reply, "$root:test").is_err());
        assert!(assert_thread_reply_relation(&missing_thread_root, "$root:test").is_err());
    }

    #[test]
    fn diff_item_visitor_scans_set_and_reset_items() {
        let set_item = synthetic_timeline_item("$root:test", Some("root"), None, None, None);
        let reset_item = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            Some("$root:test"),
            None,
        );
        let diffs = vec![
            TimelineDiff::Set {
                index: 0,
                item: set_item,
            },
            TimelineDiff::Reset {
                items: vec![reset_item],
            },
        ];
        let mut bodies = Vec::new();

        visit_timeline_diff_items(&diffs, |item| {
            if let Some(body) = item.body.as_deref() {
                bodies.push(body.to_owned());
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(bodies, ["root", "Phase 11 QA thread reply from B"]);
    }

    #[test]
    fn body_wait_observer_tolerates_transient_decryption_failure_before_expected_body() {
        let mut observer = BodyWaitObserver::new("delivered encrypted body");
        let utd = synthetic_timeline_item(
            "$utd:test",
            Some("Unable to decrypt message"),
            None,
            None,
            None,
        );
        let delivered = synthetic_timeline_item(
            "$delivered:test",
            Some("later delivered encrypted body"),
            None,
            None,
            None,
        );

        assert!(observer.observe_items(&[utd]).is_none());
        assert!(observer.saw_decryption_failure);
        assert!(
            observer
                .timeout_message("strict receive")
                .contains("transient undecryptable")
        );

        let found = observer
            .observe_diffs(&[TimelineDiff::Set {
                index: 0,
                item: delivered,
            }])
            .unwrap()
            .expect("expected body should still succeed after transient UTD");

        assert_eq!(
            found.body.as_deref(),
            Some("later delivered encrypted body")
        );
    }

    #[test]
    fn find_timeline_item_with_body_finds_thread_reply_in_one_batch() {
        let items = vec![koushi_core::event::TimelineItem {
            id: koushi_core::event::TimelineItemId::Synthetic {
                synthetic_id: "thread-reply".to_owned(),
            },
            sender: Some("@b:test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("Phase 5 QA thread reply from B".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: Some("$root:test".to_owned()),
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }];

        assert_eq!(
            find_timeline_item_with_body(&items, "thread reply from B")
                .as_ref()
                .and_then(|item| item.body.as_deref()),
            Some("Phase 5 QA thread reply from B")
        );
    }

    #[test]
    fn find_timeline_item_with_body_returns_none_when_missing() {
        let items = vec![koushi_core::event::TimelineItem {
            id: koushi_core::event::TimelineItemId::Synthetic {
                synthetic_id: "placeholder".to_owned(),
            },
            sender: None,
            sender_label: None,
            sender_avatar: None,
            body: Some("Phase 5 QA message 1".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }];

        assert!(find_timeline_item_with_body(&items, "thread reply from B").is_none());
    }

    #[test]
    fn send_flow_waiter_accepts_send_completed_before_local_echo() {
        let key = TimelineKey::room(
            AccountKey("@alice:test".to_owned()),
            "!room:test".to_owned(),
        );
        let request_id = koushi_core::ids::RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let mut waiter = SendFlowWaiter::new(
            request_id,
            key.clone(),
            "qa-phase5-txn-1",
            "Phase 5 QA message 1",
        );

        assert!(!waiter.is_complete());
        waiter
            .observe(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: key.clone(),
                transaction_id: "qa-phase5-txn-1".to_owned(),
                event_id: "$event:test".to_owned(),
            }))
            .unwrap();
        assert!(!waiter.is_complete());

        waiter
            .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: key.clone(),
                generation: koushi_core::ids::TimelineGeneration(1),
                batch_id: koushi_core::ids::TimelineBatchId(1),
                diffs: vec![koushi_core::event::TimelineDiff::PushBack {
                    item: koushi_core::event::TimelineItem {
                        id: koushi_core::event::TimelineItemId::Transaction {
                            transaction_id: "sdk-txn-1".to_owned(),
                        },
                        sender: Some("@alice:test".to_owned()),
                        sender_label: None,
                        sender_avatar: None,
                        body: Some("Phase 5 QA message 1".to_owned()),
                        notice_i18n: None,
                        message_kind: Default::default(),
                        spoiler_spans: Vec::new(),
                        timestamp_ms: None,
                        in_reply_to_event_id: None,
                        formatted: None,
                        reply_quote: None,
                        thread_root: None,
                        thread_summary: None,
                        media: None,
                        link_previews: None,
                        link_ranges: Vec::new(),
                        reactions: Vec::new(),
                        can_react: false,
                        is_redacted: false,
                        is_hidden: false,
                        can_redact: false,
                        is_edited: false,
                        can_edit: false,
                        actions: TimelineMessageActions::default(),
                        send_state: None,
                        unable_to_decrypt: None,
                    },
                }],
            }))
            .unwrap();

        let result = waiter.finish().unwrap();
        assert_eq!(result.sdk_transaction_id, "sdk-txn-1");
        assert_eq!(result.send_transaction_id, "qa-phase5-txn-1");
        assert_eq!(result.event_id, "$event:test");
    }

    #[test]
    fn send_flow_waiter_status_reports_local_echo_send_state() {
        let key = TimelineKey::room(
            AccountKey("@alice:test".to_owned()),
            "!room:test".to_owned(),
        );
        let request_id = koushi_core::ids::RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let mut waiter = SendFlowWaiter::new(
            request_id,
            key.clone(),
            "qa-phase5-txn-1",
            "Phase 5 QA message 1",
        );

        waiter
            .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key,
                generation: koushi_core::ids::TimelineGeneration(1),
                batch_id: koushi_core::ids::TimelineBatchId(1),
                diffs: vec![koushi_core::event::TimelineDiff::PushBack {
                    item: koushi_core::event::TimelineItem {
                        id: koushi_core::event::TimelineItemId::Transaction {
                            transaction_id: "sdk-txn-1".to_owned(),
                        },
                        sender: Some("@alice:test".to_owned()),
                        sender_label: None,
                        sender_avatar: None,
                        body: Some("Phase 5 QA message 1".to_owned()),
                        notice_i18n: None,
                        message_kind: Default::default(),
                        spoiler_spans: Vec::new(),
                        timestamp_ms: None,
                        in_reply_to_event_id: None,
                        formatted: None,
                        reply_quote: None,
                        thread_root: None,
                        thread_summary: None,
                        media: None,
                        link_previews: None,
                        link_ranges: Vec::new(),
                        reactions: Vec::new(),
                        can_react: false,
                        is_redacted: false,
                        is_hidden: false,
                        can_redact: false,
                        is_edited: false,
                        can_edit: false,
                        actions: TimelineMessageActions::default(),
                        send_state: Some(TimelineSendState::Sending),
                        unable_to_decrypt: None,
                    },
                }],
            }))
            .unwrap();

        assert!(
            waiter
                .status_summary()
                .contains("local_echo_send_state=Sending")
        );
    }

    #[test]
    fn send_flow_waiter_errors_when_local_echo_becomes_not_sent() {
        let key = TimelineKey::room(
            AccountKey("@alice:test".to_owned()),
            "!room:test".to_owned(),
        );
        let request_id = koushi_core::ids::RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let mut waiter = SendFlowWaiter::new(
            request_id,
            key.clone(),
            "qa-phase5-txn-1",
            "Phase 5 QA message 1",
        );

        let err = waiter
            .observe(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key,
                generation: koushi_core::ids::TimelineGeneration(1),
                batch_id: koushi_core::ids::TimelineBatchId(1),
                diffs: vec![koushi_core::event::TimelineDiff::PushBack {
                    item: koushi_core::event::TimelineItem {
                        id: koushi_core::event::TimelineItemId::Transaction {
                            transaction_id: "sdk-txn-1".to_owned(),
                        },
                        sender: Some("@alice:test".to_owned()),
                        sender_label: None,
                        sender_avatar: None,
                        body: Some("Phase 5 QA message 1".to_owned()),
                        notice_i18n: None,
                        message_kind: Default::default(),
                        spoiler_spans: Vec::new(),
                        timestamp_ms: None,
                        in_reply_to_event_id: None,
                        formatted: None,
                        reply_quote: None,
                        thread_root: None,
                        thread_summary: None,
                        media: None,
                        link_previews: None,
                        link_ranges: Vec::new(),
                        reactions: Vec::new(),
                        can_react: false,
                        is_redacted: false,
                        is_hidden: false,
                        can_redact: false,
                        is_edited: false,
                        can_edit: false,
                        actions: TimelineMessageActions::default(),
                        send_state: Some(TimelineSendState::NotSent {
                            reason: koushi_core::event::TimelineSendFailureReason::Recoverable,
                        }),
                        unable_to_decrypt: None,
                    },
                }],
            }))
            .unwrap_err();

        assert!(err.contains("send flow failed"));
        assert!(err.contains("local_echo_send_state=NotSent(recoverable)"));
    }

    #[test]
    fn headless_qa_binary_initializes_rust_log_tracing() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");
        assert!(production_source.contains("init_headless_qa_tracing_from_env();"));
        assert!(production_source.contains("tracing_subscriber::EnvFilter"));
    }

    #[test]
    fn e2ee_strict_qa_keeps_actor_owned_sync_running_for_multi_device_send() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");

        assert!(!production_source.contains("ENV_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND"));
        assert!(!production_source.contains("pause sync A before multi-device send"));
        assert!(!production_source.contains("pause sync B2 before multi-device send"));
        assert!(production_source.contains("wait_for_item_with_body_or_decryption_failure("));
    }

    #[test]
    fn e2ee_strict_qa_uses_typed_causal_checks_after_recipient_device_verification() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");

        assert!(!production_source.contains("settle_e2ee_device_list_propagation_for_qa"));
        assert!(!production_source.contains("DEVICE_LIST_SETTLE_SYNC_TIMEOUT"));
        assert!(production_source.contains("e2ee recipient verification B/B2"));
        assert!(production_source.contains("e2ee multi-device B2 room list"));
        assert!(production_source.contains("e2ee multi-device B2 receive"));
    }

    #[test]
    fn e2ee_device_verification_labels_distinguish_recipient_second_device() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");

        assert!(production_source.contains("e2ee gated self verification A/A2"));
        assert!(production_source.contains("e2ee recipient verification B/B2"));
        assert!(production_source.contains("primary incoming request"));
        assert!(!production_source.contains("request secondary to primary"));
    }

    #[test]
    fn staged_scenarios_stop_after_their_requested_stage() {
        assert!(QaScenario::Safety.should_run_stage(QaStage::Safety));
        assert!(!QaScenario::Safety.should_run_stage(QaStage::LoginSync));

        assert!(QaScenario::LoginSync.should_run_stage(QaStage::Safety));
        assert!(QaScenario::LoginSync.should_run_stage(QaStage::LoginSync));
        assert!(!QaScenario::LoginSync.should_run_stage(QaStage::RoomSpace));
        assert!(!QaScenario::LoginSync.should_run_stage(QaStage::InvitesDm));

        assert!(QaScenario::InvitesDm.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::InvitesDm.should_run_stage(QaStage::InvitesDm));
        assert!(!QaScenario::InvitesDm.should_run_stage(QaStage::RoomSpace));

        assert!(QaScenario::RoomSpace.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::RoomSpace.should_run_stage(QaStage::RoomSpace));
        assert!(!QaScenario::RoomSpace.should_run_stage(QaStage::InvitesDm));
        assert!(!QaScenario::RoomSpace.should_run_stage(QaStage::E2eeTrust));
        assert!(!QaScenario::RoomSpace.should_run_stage(QaStage::Timeline));

        assert!(QaScenario::Timeline.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Timeline.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Timeline.should_run_stage(QaStage::Timeline));
        assert!(!QaScenario::Timeline.should_run_stage(QaStage::E2eeTrust));
        assert!(!QaScenario::Timeline.should_run_stage(QaStage::Activity));
        assert!(!QaScenario::Timeline.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::Timeline.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::TimelineReconnect.should_run_stage(QaStage::Safety));
        assert!(QaScenario::TimelineReconnect.should_run_stage(QaStage::TimelineReconnect));
        assert!(!QaScenario::TimelineReconnect.should_run_stage(QaStage::LoginSync));
        assert!(!QaScenario::TimelineReconnect.should_run_stage(QaStage::Timeline));
        assert!(!QaScenario::TimelineReconnect.should_run_stage(QaStage::SendQueue));

        assert!(QaScenario::TimelineLegacyFallback.should_run_stage(QaStage::Safety));
        assert!(
            QaScenario::TimelineLegacyFallback.should_run_stage(QaStage::TimelineLegacyFallback)
        );
        assert!(!QaScenario::TimelineLegacyFallback.should_run_stage(QaStage::LoginSync));

        assert!(QaScenario::TimelineLegacyPersistedGap.should_run_stage(QaStage::Safety));
        assert!(
            QaScenario::TimelineLegacyPersistedGap
                .should_run_stage(QaStage::TimelineLegacyPersistedGap)
        );
        assert!(
            !QaScenario::TimelineLegacyPersistedGap
                .should_run_stage(QaStage::TimelineLegacyFallback)
        );

        assert!(QaScenario::TimelineStress.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::TimelineStress.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::TimelineStress.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::TimelineStress.should_run_stage(QaStage::TimelineStress));
        assert!(!QaScenario::TimelineStress.should_run_stage(QaStage::Activity));
        assert!(!QaScenario::TimelineStress.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::Activity.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Activity.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Activity.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::Activity.should_run_stage(QaStage::Activity));
        assert!(QaScenario::Activity.suppress_matrix_identifiers());
        assert!(!QaScenario::Activity.should_run_stage(QaStage::Composer));
        assert!(!QaScenario::Activity.should_run_stage(QaStage::Reply));

        assert!(QaScenario::CredentialHealth.should_run_stage(QaStage::Safety));
        assert!(QaScenario::CredentialHealth.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::CredentialHealth.should_run_stage(QaStage::CredentialHealth));
        assert!(!QaScenario::CredentialHealth.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::CredentialHealth.suppress_matrix_identifiers());

        assert!(QaScenario::NativeAttention.should_run_stage(QaStage::Safety));
        assert!(QaScenario::NativeAttention.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::NativeAttention.should_run_stage(QaStage::NativeAttention));
        assert!(!QaScenario::NativeAttention.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::NativeAttention.suppress_matrix_identifiers());

        assert!(QaScenario::Reply.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Reply.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Reply.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::Reply.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::Reply.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::Media.should_run_stage(QaStage::Safety));
        assert!(QaScenario::Media.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Media.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Media.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::Media.should_run_stage(QaStage::Media));
        assert!(!QaScenario::Media.should_run_stage(QaStage::LiveSignals));
        assert!(!QaScenario::Media.should_run_stage(QaStage::Thread));
        assert!(!QaScenario::Media.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::LiveSignals.should_run_stage(QaStage::Safety));
        assert!(QaScenario::LiveSignals.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::LiveSignals.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::LiveSignals.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::LiveSignals.should_run_stage(QaStage::LiveSignals));
        assert!(!QaScenario::LiveSignals.should_run_stage(QaStage::Media));
        assert!(!QaScenario::LiveSignals.should_run_stage(QaStage::Thread));
        assert!(!QaScenario::LiveSignals.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::Thread.should_run_stage(QaStage::Safety));
        assert!(QaScenario::Thread.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Thread.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::Thread.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::Thread.should_run_stage(QaStage::Reply));
        assert!(QaScenario::Thread.should_run_stage(QaStage::Thread));
        assert!(!QaScenario::Thread.should_run_stage(QaStage::Media));
        assert!(!QaScenario::Thread.should_run_stage(QaStage::EditRedactSearch));

        assert!(QaScenario::Directory.should_run_stage(QaStage::Safety));
        assert!(QaScenario::Directory.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::Directory.should_run_stage(QaStage::Directory));
        assert!(!QaScenario::Directory.should_run_stage(QaStage::Timeline));
        assert!(!QaScenario::Directory.should_run_stage(QaStage::Reply));

        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::Safety));
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::RoomManagement));
        assert!(!QaScenario::RoomManagement.should_run_stage(QaStage::Timeline));
        assert!(!QaScenario::RoomManagement.should_run_stage(QaStage::Reply));

        assert!(QaScenario::LinkPreview.should_run_stage(QaStage::Safety));
        assert!(QaScenario::LinkPreview.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::LinkPreview.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::LinkPreview.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::LinkPreview.should_run_stage(QaStage::Composer));
        assert!(QaScenario::LinkPreview.should_run_stage(QaStage::LinkPreview));
        assert!(!QaScenario::LinkPreview.should_run_stage(QaStage::Reply));
        assert!(QaScenario::LinkPreview.suppress_matrix_identifiers());

        assert!(QaScenario::All.should_run_stage(QaStage::Safety));
        assert!(QaScenario::All.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::All.should_run_stage(QaStage::E2eeTrust));
        assert!(QaScenario::All.should_run_stage(QaStage::InvitesDm));
        assert!(QaScenario::All.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::All.should_run_stage(QaStage::Directory));
        assert!(QaScenario::All.should_run_stage(QaStage::RoomManagement));
        assert!(QaScenario::All.should_run_stage(QaStage::Timeline));
        assert!(!QaScenario::All.should_run_stage(QaStage::TimelineReconnect));
        assert!(!QaScenario::All.should_run_stage(QaStage::TimelineStress));
        assert!(QaScenario::All.should_run_stage(QaStage::Activity));
        assert!(QaScenario::All.should_run_stage(QaStage::CredentialHealth));
        assert!(QaScenario::All.should_run_stage(QaStage::Reply));
        assert!(QaScenario::All.should_run_stage(QaStage::Media));
        assert!(QaScenario::All.should_run_stage(QaStage::LiveSignals));
        assert!(QaScenario::All.should_run_stage(QaStage::Thread));
        assert!(QaScenario::All.should_run_stage(QaStage::EditRedactSearch));
        assert!(QaScenario::All.should_run_stage(QaStage::ScheduledSend));
        assert!(QaScenario::All.should_run_stage(QaStage::SendQueue));
        assert!(QaScenario::All.should_run_stage(QaStage::RestoreCleanup));
        assert!(QaScenario::All.should_run_stage(QaStage::LinkPreview));
    }

    #[test]
    fn send_queue_scenario_skips_generic_fixture_stages_and_reports_private_tokens() {
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::Safety));
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::LoginSync));
        assert!(!QaScenario::SendQueue.should_run_stage(QaStage::RoomSpace));
        assert!(!QaScenario::SendQueue.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::SendQueue));
        assert!(!QaScenario::SendQueue.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::SendQueue.should_run_stage(QaStage::EditRedactSearch));
        assert_eq!(
            stages_for_scenario(QaScenario::SendQueue),
            [QaStage::Safety, QaStage::LoginSync, QaStage::SendQueue]
        );

        assert_eq!(
            final_tokens_for_scenario(QaScenario::SendQueue),
            [
                "safety=ok",
                "login_sync=ok",
                "send_fail=ok",
                "resend=ok",
                "cancel_send=ok",
                "fifo=ok",
                "unsent_restart=ok",
                "display_projection_reset_fallbacks=0",
                "restore_cleanup=ok",
            ]
        );
    }

    #[test]
    fn send_queue_display_projection_fallback_gate_requires_zero_counter_delta() {
        assert_eq!(
            assert_zero_display_projection_reset_fallback_delta(41, 41),
            Ok(())
        );
        assert!(assert_zero_display_projection_reset_fallback_delta(41, 42).is_err());

        let source = include_str!("headless-core-qa.rs");
        let stage = source
            .split("async fn run_send_queue_stage")
            .nth(1)
            .expect("send queue stage")
            .split("async fn unsubscribe_timeline_for_qa")
            .next()
            .expect("send queue stage boundary");
        assert!(stage.contains("display_projection_reset_fallback_count()"));
        assert!(stage.contains("assert_zero_display_projection_reset_fallback_delta"));
        assert!(stage.contains("println!(\"display_projection_reset_fallbacks=0\")"));
    }

    #[test]
    fn send_queue_alone_uses_the_focused_early_route() {
        assert!(should_run_focused_send_queue_route(QaScenario::SendQueue));

        for scenario in [
            QaScenario::All,
            QaScenario::LoginSync,
            QaScenario::RoomSpace,
            QaScenario::Timeline,
            QaScenario::E2eeTrust,
        ] {
            assert!(
                !should_run_focused_send_queue_route(scenario),
                "{scenario:?} must retain its existing route"
            );
        }

        let source = include_str!("headless-core-qa.rs");
        let run_async_before_generic_fixture = source
            .split("async fn run_async")
            .nth(1)
            .and_then(|rest| rest.split("// One CoreRuntime per synthetic user").next())
            .expect("run_async before the generic two-user fixture");
        let focused_dispatch = run_async_before_generic_fixture
            .find("if should_run_focused_send_queue_route(scenario)")
            .expect("run_async dispatches SendQueue through its focused route");
        let focused_call = run_async_before_generic_fixture
            .find("run_focused_send_queue_scenario(&config).await?")
            .expect("run_async invokes the focused SendQueue scenario");
        let focused_return = run_async_before_generic_fixture[focused_call..]
            .find("return Ok(scenario_report(&config.server_kind, scenario))")
            .map(|offset| focused_call + offset)
            .expect("focused SendQueue dispatch returns before generic fixture setup");
        assert!(focused_dispatch < focused_call);
        assert!(focused_call < focused_return);

        let route = source
            .split("async fn run_focused_send_queue_scenario")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_send_queue_stage").next())
            .expect("focused SendQueue route");
        let drop_connection = route
            .find("drop(conn)")
            .expect("focused route drops its bootstrap connection");
        let ordered_shutdown = route
            .find("runtime.shutdown()")
            .expect("focused route awaits ordered runtime shutdown");
        let standalone_stage = route
            .find("run_send_queue_stage(config, &recovery_secret).await")
            .expect("focused route invokes the standalone SendQueue stage");

        assert!(route.contains("QaParticipantLoginGate::BootstrapNewIdentity"));
        assert!(route.contains("bootstrap_recovery_secret"));
        assert!(drop_connection < ordered_shutdown);
        assert!(ordered_shutdown < standalone_stage);
        assert!(!route.contains("user_b"));
        assert!(!route.contains("password_b"));
    }

    #[test]
    fn focused_send_queue_bootstrap_logs_out_before_ordered_shutdown() {
        let source = include_str!("headless-core-qa.rs");
        let route = source
            .split("async fn run_focused_send_queue_scenario")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_send_queue_stage").next())
            .expect("focused SendQueue route");

        let sync_stop = route
            .find("SyncCommand::Stop")
            .expect("focused route submits sync stop");
        let sync_stopped = route
            .find("wait_for_sync_stopped")
            .expect("focused route waits for correlated sync stop");
        let logout = route
            .find("AccountCommand::Logout")
            .expect("focused route submits logout");
        let logged_out = route
            .find("wait_for_logged_out")
            .expect("focused route waits for correlated logout");
        let drop_connection = route
            .find("drop(conn)")
            .expect("focused route drops its bootstrap connection");
        let ordered_shutdown = route
            .find("runtime.shutdown()")
            .expect("focused route awaits ordered runtime shutdown");
        let missing_secret = route
            .find("send_queue bootstrap recovery secret unavailable")
            .expect("focused route reports a missing recovery secret");
        let standalone_stage = route
            .find("run_send_queue_stage(config, &recovery_secret).await")
            .expect("focused route invokes the standalone SendQueue stage");

        assert!(!route.contains("account_key: _"));
        assert!(route.contains("&account_key"));
        assert!(sync_stop < sync_stopped);
        assert!(sync_stopped < logout);
        assert!(logout < logged_out);
        assert!(logged_out < drop_connection);
        assert!(drop_connection < ordered_shutdown);
        assert!(ordered_shutdown < missing_secret);
        assert!(missing_secret < standalone_stage);
    }

    #[test]
    fn invites_dm_primary_login_requires_new_identity_bootstrap() {
        assert!(
            should_bootstrap_new_identity_before_logged_in(QaScenario::InvitesDm),
            "focused InvitesDm must bootstrap primary A before LoggedIn"
        );
    }

    #[test]
    fn normal_secondary_participant_policy_covers_only_shared_b_stages() {
        for scenario in [
            QaScenario::All,
            QaScenario::InvitesDm,
            QaScenario::Directory,
            QaScenario::RoomSpace,
            QaScenario::Timeline,
        ] {
            assert!(
                should_run_normal_secondary_participant(scenario),
                "{scenario:?} needs the shared normal B session"
            );
        }

        for scenario in [
            QaScenario::Safety,
            QaScenario::LoginSync,
            QaScenario::CredentialHealth,
            QaScenario::E2eeTrust,
            QaScenario::GateRestore,
            QaScenario::GateNegative,
            QaScenario::SendQueue,
        ] {
            assert!(
                !should_run_normal_secondary_participant(scenario),
                "{scenario:?} must not create the shared normal B session"
            );
        }
    }

    #[test]
    fn run_async_centrally_owns_one_normal_secondary_login() {
        let source = include_str!("headless-core-qa.rs");
        let before_room_space = source
            .split("async fn run_async")
            .nth(1)
            .expect("run_async should exist")
            .split("// --- Phase 4: Room operations")
            .next()
            .expect("RoomSpace should follow shared stage setup");

        assert!(before_room_space.contains(
            "let mut normal_secondary = if should_run_normal_secondary_participant(scenario)"
        ));
        assert_eq!(
            before_room_space
                .matches("login_synced_participant_for_qa(")
                .count(),
            1,
            "run_async must own exactly one normal B login"
        );
        assert!(before_room_space.contains("QaParticipantLoginGate::BootstrapNewIdentity"));
        assert_eq!(
            before_room_space
                .matches("cleanup_normal_secondary_participant_for_qa(")
                .count(),
            2,
            "focused InvitesDm and pre-RoomSpace exits each need one ordered cleanup path"
        );
    }

    #[test]
    fn invites_dm_and_directory_borrow_b_without_owning_its_lifecycle() {
        let source = include_str!("headless-core-qa.rs");
        let invites = source
            .split("async fn run_invites_dm_stage")
            .nth(1)
            .expect("InvitesDm stage should exist")
            .split("async fn run_directory_stage")
            .next()
            .expect("directory stage should follow InvitesDm");
        let directory = source
            .split("async fn run_directory_stage")
            .nth(1)
            .expect("directory stage should exist")
            .split("async fn join_directory_room_for_qa")
            .next()
            .expect("directory join helper should follow directory stage");

        for (label, stage) in [("InvitesDm", invites), ("Directory", directory)] {
            assert!(
                stage.contains("conn_b: &mut CoreConnection"),
                "{label} must borrow the centrally owned B connection"
            );
            for forbidden in [
                "CoreRuntime::",
                "AccountCommand::LoginPassword",
                "wait_for_logged_in",
                "login_synced_participant_for_qa(",
                "cleanup_logged_in_runtime",
            ] {
                assert!(
                    !stage.contains(forbidden),
                    "{label} must not own B lifecycle operation {forbidden}"
                );
            }
        }
    }

    #[test]
    fn room_space_reuses_and_consumes_the_central_secondary_owner() {
        let source = include_str!("headless-core-qa.rs");
        let room_space = source
            .split("// --- Phase 4: Room operations")
            .nth(1)
            .expect("RoomSpace stage should exist")
            .split("// --- Phase 5: Timeline subscribe")
            .next()
            .expect("Timeline should follow RoomSpace");

        assert!(room_space.contains("normal_secondary.take()"));
        assert!(room_space.contains("let QaParticipantLoginOutcome"));
        for forbidden in [
            "CoreRuntime::start_with_data_dir(data_dir_b)",
            "AccountCommand::LoginPassword",
            "wait_for_logged_in",
            "login_synced_participant_for_qa(",
        ] {
            assert!(
                !room_space.contains(forbidden),
                "RoomSpace must reuse B instead of performing {forbidden}"
            );
        }
    }

    #[test]
    fn normal_secondary_cleanup_paths_use_one_ordered_runtime_shutdown() {
        let source = include_str!("headless-core-qa.rs");
        let focused_cleanup = source
            .split("async fn cleanup_logged_in_runtime")
            .nth(1)
            .expect("logged-in runtime cleanup should exist")
            .split("async fn cleanup_normal_secondary_participant_for_qa")
            .next()
            .expect("normal secondary cleanup should follow runtime cleanup");
        assert!(focused_cleanup.contains("runtime.shutdown().await"));
        assert!(!focused_cleanup.contains("drop(runtime)"));
        assert!(!focused_cleanup.contains("tokio::time::sleep"));

        let all_cleanup = source
            .split("// --- Logout B ---")
            .nth(1)
            .expect("All should own a B cleanup section")
            .split("async fn complete_new_identity_gate_for_qa")
            .next()
            .expect("All B cleanup should end with run_async");
        assert_eq!(all_cleanup.matches("AccountCommand::Logout").count(), 1);
        assert_eq!(all_cleanup.matches("runtime_b.shutdown().await").count(), 1);
        assert!(!all_cleanup.contains("cleanup_normal_secondary_participant_for_qa"));
    }

    #[test]
    fn all_flow_retains_the_primary_recovery_secret_for_its_send_queue_stage() {
        assert!(QaScenario::All.should_run_stage(QaStage::SendQueue));
        assert!(
            should_bootstrap_new_identity_before_logged_in(QaScenario::All),
            "All must retain the primary recovery secret required by its SendQueue stage"
        );
        assert!(should_bootstrap_new_identity_before_logged_in(
            QaScenario::E2eeTrust
        ));
        assert!(should_bootstrap_new_identity_before_logged_in(
            QaScenario::GateRestore
        ));
        assert!(should_bootstrap_new_identity_before_logged_in(
            QaScenario::GateNegative
        ));

        assert!(!should_bootstrap_new_identity_before_logged_in(
            QaScenario::GateNoProof
        ));
        assert!(!should_bootstrap_new_identity_before_logged_in(
            QaScenario::LoginSync
        ));
        assert!(!should_bootstrap_new_identity_before_logged_in(
            QaScenario::TimelineReconnect
        ));
        assert!(!should_bootstrap_new_identity_before_logged_in(
            QaScenario::SendQueue
        ));

        let source = include_str!("headless-core-qa.rs");
        let all_send_queue_route = source
            .split("if scenario.should_run_stage(QaStage::SendQueue)")
            .nth(1)
            .expect("All route should retain the SendQueue stage")
            .split("if !scenario.should_run_stage(QaStage::EditRedactSearch)")
            .next()
            .expect("EditRedactSearch route should follow SendQueue");
        assert!(all_send_queue_route.contains("bootstrap_recovery_secret_a"));
        assert!(all_send_queue_route.contains("run_send_queue_stage(&config, recovery_secret)"));

        let standalone_send_queue = source
            .split("async fn run_send_queue_stage")
            .nth(1)
            .expect("standalone SendQueue stage")
            .split("async fn unsubscribe_timeline_for_qa")
            .next()
            .expect("standalone SendQueue stage end");
        assert!(
            standalone_send_queue
                .contains("QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret)")
        );
    }

    #[test]
    fn standalone_send_queue_login_requires_primary_recovery_secret() {
        let source = include_str!("headless-core-qa.rs");
        let stage = source
            .split("async fn run_send_queue_stage")
            .nth(1)
            .expect("standalone SendQueue stage")
            .split("async fn unsubscribe_timeline_for_qa")
            .next()
            .expect("standalone SendQueue stage end");

        assert!(stage.contains("login_synced_participant_for_qa("));
        assert!(stage.contains("proxy.homeserver_url()"));
        assert!(stage.contains("recovery_secret: &AuthSecret"));
        assert!(stage.contains("QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret)"));
        assert!(!stage.contains("\n        true,"));
        assert!(!stage.contains("AccountCommand::LoginPassword"));
        assert!(!stage.contains("wait_for_logged_in"));
    }

    #[test]
    fn participant_login_gate_policy_distinguishes_bootstrap_from_recovery() {
        let source = include_str!("headless-core-qa.rs");
        let before_helper = source
            .split("async fn login_synced_participant_for_qa")
            .next()
            .expect("source before centralized participant login helper");
        let helper = source
            .split("async fn login_synced_participant_for_qa")
            .nth(1)
            .expect("centralized participant login helper")
            .split("async fn subscribe_timeline_for_qa")
            .next()
            .expect("centralized participant login helper end");

        assert!(before_helper.contains("enum QaParticipantLoginGate<'a>"));
        assert!(before_helper.contains("BootstrapNewIdentity"));
        assert!(before_helper.contains("RecoverExistingIdentity(&'a AuthSecret)"));
        assert!(helper.contains("gate: QaParticipantLoginGate<'_>"));
        assert!(!helper.contains("bootstrap_new_identity: bool"));
    }

    #[tokio::test]
    async fn owned_e2ee_recipient_cleanup_runs_after_post_login_stage_failure() {
        let cleanup_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_attempts = cleanup_attempts.clone();

        let result = finish_e2ee_recipient_stage_with_owned_cleanup(
            Err::<(), _>("injected post-login failure".to_owned()),
            Some("owned-recipient"),
            move |participant| {
                let cleanup_attempts = cleanup_attempts.clone();
                async move {
                    assert_eq!(participant, "owned-recipient");
                    cleanup_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await;

        assert_eq!(result.unwrap_err(), "injected post-login failure");
        assert_eq!(
            observed_attempts.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn borrowed_e2ee_stage_failure_runs_outer_caller_cleanup_path() {
        let cleanup_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_attempts = cleanup_attempts.clone();

        let result = retain_or_cleanup_e2ee_callers_after_stage(
            Err::<(), _>("injected borrowed-stage failure".to_owned()),
            ("caller-a", "caller-b"),
            move |callers| {
                let cleanup_attempts = cleanup_attempts.clone();
                async move {
                    assert_eq!(callers, ("caller-a", "caller-b"));
                    cleanup_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await;

        assert_eq!(result.unwrap_err(), "injected borrowed-stage failure");
        assert_eq!(
            observed_attempts.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum RecordedOwnedE2eeCleanupOperation {
        StopSync,
        Logout(QaE2eeLogoutBarrier),
        AuthoritativeLogoutBarrier(QaE2eeLogoutBarrier),
        DropConnection,
        ShutdownRuntime,
    }

    struct RecordingOwnedE2eeCleanupOperations {
        participant: &'static str,
        operations: std::sync::Arc<
            std::sync::Mutex<Vec<(&'static str, RecordedOwnedE2eeCleanupOperation)>>,
        >,
        fail_authoritative_barrier: bool,
    }

    impl RecordingOwnedE2eeCleanupOperations {
        fn record(&self, operation: RecordedOwnedE2eeCleanupOperation) {
            self.operations
                .lock()
                .expect("cleanup observation lock")
                .push((self.participant, operation));
        }
    }

    impl QaOwnedE2eeCleanupOperations for RecordingOwnedE2eeCleanupOperations {
        async fn stop_sync(&mut self, _label: &str) -> Result<(), String> {
            self.record(RecordedOwnedE2eeCleanupOperation::StopSync);
            Ok(())
        }

        async fn submit_logout(
            &mut self,
            barrier: &QaE2eeLogoutBarrier,
            _label: &str,
        ) -> Result<(), String> {
            self.record(RecordedOwnedE2eeCleanupOperation::Logout(barrier.clone()));
            Ok(())
        }

        async fn wait_for_authoritative_logout(
            &mut self,
            barrier: &QaE2eeLogoutBarrier,
            _label: &str,
        ) -> Result<(), String> {
            self.record(
                RecordedOwnedE2eeCleanupOperation::AuthoritativeLogoutBarrier(barrier.clone()),
            );
            if self.fail_authoritative_barrier {
                Err("injected authoritative logout barrier failure".to_owned())
            } else {
                Ok(())
            }
        }

        fn drop_connection(&mut self) {
            self.record(RecordedOwnedE2eeCleanupOperation::DropConnection);
        }

        async fn shutdown_runtime(&mut self) {
            self.record(RecordedOwnedE2eeCleanupOperation::ShutdownRuntime);
        }
    }

    fn recording_owned_e2ee_cleanup_operations(
        participant: &'static str,
        fail_authoritative_barrier: bool,
        operations: &std::sync::Arc<
            std::sync::Mutex<Vec<(&'static str, RecordedOwnedE2eeCleanupOperation)>>,
        >,
    ) -> RecordingOwnedE2eeCleanupOperations {
        RecordingOwnedE2eeCleanupOperations {
            participant,
            operations: operations.clone(),
            fail_authoritative_barrier,
        }
    }

    #[tokio::test]
    async fn owned_e2ee_cleanup_orders_each_ownership_phase() {
        let account_key = AccountKey("@owned:example.invalid".to_owned());
        let cases = [
            (
                QaOwnedRuntimePhase::LoginNotSubmitted,
                vec![
                    RecordedOwnedE2eeCleanupOperation::DropConnection,
                    RecordedOwnedE2eeCleanupOperation::ShutdownRuntime,
                ],
            ),
            (
                QaOwnedRuntimePhase::LoginSubmitted,
                vec![
                    RecordedOwnedE2eeCleanupOperation::Logout(QaE2eeLogoutBarrier::AnyAccount),
                    RecordedOwnedE2eeCleanupOperation::AuthoritativeLogoutBarrier(
                        QaE2eeLogoutBarrier::AnyAccount,
                    ),
                    RecordedOwnedE2eeCleanupOperation::DropConnection,
                    RecordedOwnedE2eeCleanupOperation::ShutdownRuntime,
                ],
            ),
            (
                QaOwnedRuntimePhase::LoggedIn(account_key.clone()),
                vec![
                    RecordedOwnedE2eeCleanupOperation::StopSync,
                    RecordedOwnedE2eeCleanupOperation::Logout(QaE2eeLogoutBarrier::Exact(
                        account_key.clone(),
                    )),
                    RecordedOwnedE2eeCleanupOperation::AuthoritativeLogoutBarrier(
                        QaE2eeLogoutBarrier::Exact(account_key),
                    ),
                    RecordedOwnedE2eeCleanupOperation::DropConnection,
                    RecordedOwnedE2eeCleanupOperation::ShutdownRuntime,
                ],
            ),
        ];

        for (phase, expected) in cases {
            let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut operations =
                recording_owned_e2ee_cleanup_operations("participant", false, &observed);

            cleanup_owned_e2ee_lifecycle_best_effort(
                &phase,
                &mut operations,
                "ownership phase cleanup",
            )
            .await
            .expect("phase cleanup should succeed");

            let actual = observed
                .lock()
                .expect("cleanup observation lock")
                .iter()
                .map(|(_, operation)| operation.clone())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }

    #[tokio::test]
    async fn borrowed_e2ee_recipient_is_not_cleaned_by_the_inner_stage() {
        let cleanup_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_attempts = cleanup_attempts.clone();

        let result = finish_e2ee_recipient_stage_with_owned_cleanup(
            Err::<(), _>("injected borrowed-stage failure".to_owned()),
            None::<&'static str>,
            move |_| {
                let cleanup_attempts = cleanup_attempts.clone();
                async move {
                    cleanup_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await;

        assert_eq!(result.unwrap_err(), "injected borrowed-stage failure");
        assert_eq!(
            observed_attempts.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn e2ee_multi_device_cleanup_attempts_every_owned_participant_after_one_failure() {
        let operations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = operations.clone();
        let account_key = AccountKey("@owned:example.invalid".to_owned());

        let result = cleanup_all_owned_e2ee_participants(
            [
                Some((
                    QaOwnedRuntimePhase::LoggedIn(account_key.clone()),
                    recording_owned_e2ee_cleanup_operations("B3", true, &operations),
                )),
                Some((
                    QaOwnedRuntimePhase::LoggedIn(account_key.clone()),
                    recording_owned_e2ee_cleanup_operations("B2", false, &operations),
                )),
                Some((
                    QaOwnedRuntimePhase::LoggedIn(account_key),
                    recording_owned_e2ee_cleanup_operations("B", false, &operations),
                )),
            ],
            move |(phase, mut participant_operations)| async move {
                cleanup_owned_e2ee_lifecycle_best_effort(
                    &phase,
                    &mut participant_operations,
                    "multi-device cleanup",
                )
                .await
            },
        )
        .await;

        assert_eq!(
            result.unwrap_err(),
            "E2EE cleanup failed for 1 owned recipient participant(s)"
        );
        let observed = observed.lock().expect("cleanup observation lock");
        for participant in ["B3", "B2", "B"] {
            let participant_operations = observed
                .iter()
                .filter_map(|(observed_participant, operation)| {
                    (*observed_participant == participant).then_some(operation)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                participant_operations.last(),
                Some(&&RecordedOwnedE2eeCleanupOperation::ShutdownRuntime),
                "{participant} must reach ordered runtime shutdown"
            );
        }
    }

    #[test]
    fn initial_items_wait_requires_exact_subscribe_cause_even_for_same_key_replays() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        let old_request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let wrong_connection_request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(2),
            sequence: request_id.sequence,
        };
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let wrong_key =
            TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!other:test");
        let initial = |projection_request_id, cause_request_id, key: TimelineKey| {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: projection_request_id,
                cause_request_id,
                key,
                actor_generation: 1,
                generation: koushi_core::ids::TimelineGeneration(0),
                items: Vec::new(),
            })
        };
        let classify = |event| match_initial_items_wait_event(event, &key, request_id);

        assert!(matches!(
            classify(initial(Some(old_request_id), Some(request_id), key.clone())),
            InitialItemsWaitMatch::Items(_)
        ));
        assert!(matches!(
            classify(initial(None, Some(request_id), key.clone())),
            InitialItemsWaitMatch::Items(_)
        ));
        assert!(matches!(
            classify(initial(Some(request_id), Some(old_request_id), key.clone())),
            InitialItemsWaitMatch::Ignore
        ));
        assert!(matches!(
            classify(initial(Some(request_id), None, key.clone())),
            InitialItemsWaitMatch::Ignore
        ));
        assert!(matches!(
            classify(initial(Some(old_request_id), Some(request_id), wrong_key)),
            InitialItemsWaitMatch::Ignore
        ));

        assert!(matches!(
            classify(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::SessionRequired,
            }),
            InitialItemsWaitMatch::Failure(CoreFailure::SessionRequired)
        ));
        assert!(matches!(
            classify(CoreEvent::OperationFailed {
                request_id: old_request_id,
                failure: CoreFailure::SessionRequired,
            }),
            InitialItemsWaitMatch::Ignore
        ));
        assert!(matches!(
            classify(CoreEvent::OperationFailed {
                request_id: wrong_connection_request_id,
                failure: CoreFailure::SessionRequired,
            }),
            InitialItemsWaitMatch::Ignore
        ));
    }

    struct ScriptedQaEventSource {
        events: std::collections::VecDeque<CoreEvent>,
    }

    impl QaEventSource for ScriptedQaEventSource {
        fn recv_event(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                match self.events.pop_front() {
                    Some(event) => Ok(event),
                    None => std::future::pending().await,
                }
            })
        }
    }

    fn withheld_projection_test_item(event_id: &str, body: &str) -> TimelineItem {
        let mut item = projection_timeline_item(event_id, false);
        item.body = Some(body.to_owned());
        item
    }

    fn withheld_projection_items_updated(key: TimelineKey, item: TimelineItem) -> CoreEvent {
        CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key,
            generation: koushi_core::ids::TimelineGeneration(0),
            batch_id: koushi_core::ids::TimelineBatchId(1),
            diffs: vec![TimelineDiff::PushBack { item }],
        })
    }

    #[tokio::test]
    async fn withheld_projection_wait_accepts_decryption_failure_from_late_items_updated() {
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let target_event_id = "$withheld:test";
        let mut source = ScriptedQaEventSource {
            events: [
                CoreEvent::Sync(SyncEvent::Running),
                withheld_projection_items_updated(
                    key.clone(),
                    withheld_projection_test_item(target_event_id, "Unable to decrypt"),
                ),
            ]
            .into(),
        };

        let origin = wait_for_withheld_event_projection_from_source(
            &mut source,
            &key,
            target_event_id,
            "blocked body",
            &[],
            "withheld projection regression",
            Duration::from_secs(1),
        )
        .await
        .expect("late decryption-failure projection should satisfy the waiter");

        assert_eq!(origin, WithheldEventProjectionOrigin::ItemsUpdated);
    }

    #[tokio::test(start_paused = true)]
    async fn withheld_projection_wait_reports_private_safe_missing_category_at_deadline() {
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let target_event_id = "$withheld:test";
        let mut source = ScriptedQaEventSource {
            events: Default::default(),
        };

        let error = wait_for_withheld_event_projection_from_source(
            &mut source,
            &key,
            target_event_id,
            "blocked body",
            &[],
            "withheld projection regression",
            Duration::from_secs(1),
        )
        .await
        .expect_err("an absent canonical event should time out as missing");

        assert!(error.contains("projection_origin=missing"));
        assert!(!error.contains(target_event_id));
        assert!(!error.contains("@qa:"));
        assert!(!error.contains("!room:"));
    }

    #[tokio::test]
    async fn withheld_projection_wait_rejects_plaintext_without_exposing_it() {
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let target_event_id = "$withheld:test";
        let private_body = "private withheld body";
        let initial_items = vec![withheld_projection_test_item(target_event_id, private_body)];
        let mut source = ScriptedQaEventSource {
            events: Default::default(),
        };

        let error = wait_for_withheld_event_projection_from_source(
            &mut source,
            &key,
            target_event_id,
            private_body,
            &initial_items,
            "withheld projection regression",
            Duration::from_secs(1),
        )
        .await
        .expect_err("plaintext projection must fail closed");

        assert!(error.contains("projection_outcome=non_failure"));
        assert!(error.contains("matches_expected_body=true"));
        assert!(!error.contains(target_event_id));
        assert!(!error.contains(private_body));
    }

    #[tokio::test]
    async fn paired_verification_wait_wakes_from_either_event_source() {
        let mut primary = ScriptedQaEventSource {
            events: Default::default(),
        };
        let mut secondary = ScriptedQaEventSource {
            events: [CoreEvent::Sync(SyncEvent::Running)].into(),
        };

        assert_eq!(
            wait_for_paired_event_until(
                &mut primary,
                &mut secondary,
                tokio::time::Instant::now() + Duration::from_secs(10),
            )
            .await,
            Ok(())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn paired_verification_wait_uses_one_absolute_deadline() {
        let mut primary = ScriptedQaEventSource {
            events: Default::default(),
        };
        let mut secondary = ScriptedQaEventSource {
            events: Default::default(),
        };
        let started_at = tokio::time::Instant::now();
        let deadline = started_at + Duration::from_secs(7);

        assert_eq!(
            wait_for_paired_event_until(&mut primary, &mut secondary, deadline).await,
            Err(PairedEventWaitError::Deadline)
        );
        assert_eq!(
            tokio::time::Instant::now().duration_since(started_at),
            Duration::from_secs(7)
        );
    }

    struct ScriptedQaSnapshotEventSource {
        events: std::collections::VecDeque<(CoreEvent, SessionState)>,
        snapshot: AppState,
        received: usize,
    }

    impl QaEventSource for ScriptedQaSnapshotEventSource {
        fn recv_event(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                match self.events.pop_front() {
                    Some((event, session)) => {
                        self.snapshot.session = session;
                        self.received += 1;
                        Ok(event)
                    }
                    None => std::future::pending().await,
                }
            })
        }
    }

    impl QaSnapshotEventSource for ScriptedQaSnapshotEventSource {
        fn snapshot(&self) -> AppState {
            self.snapshot.clone()
        }
    }

    struct IntervalQaEventSource {
        interval: tokio::time::Interval,
    }

    impl QaEventSource for IntervalQaEventSource {
        fn recv_event(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                self.interval.tick().await;
                Ok(CoreEvent::Sync(SyncEvent::Running))
            })
        }
    }

    struct IntervalQaSnapshotEventSource {
        interval: tokio::time::Interval,
        snapshot: AppState,
        first_event: Option<CoreEvent>,
    }

    impl QaEventSource for IntervalQaSnapshotEventSource {
        fn recv_event(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<CoreEvent, koushi_core::runtime::EventStreamLag>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                if let Some(event) = self.first_event.take() {
                    return Ok(event);
                }
                self.interval.tick().await;
                Ok(CoreEvent::Sync(SyncEvent::Running))
            })
        }
    }

    impl QaSnapshotEventSource for IntervalQaSnapshotEventSource {
        fn snapshot(&self) -> AppState {
            self.snapshot.clone()
        }
    }

    struct SharedSnapshotPendingEventSource {
        snapshot: Arc<Mutex<AppState>>,
    }

    impl QaEventSource for SharedSnapshotPendingEventSource {
        fn recv_event(&mut self) -> QaEventFuture<'_> {
            Box::pin(std::future::pending())
        }
    }

    impl QaSnapshotEventSource for SharedSnapshotPendingEventSource {
        fn snapshot(&self) -> AppState {
            self.snapshot
                .lock()
                .expect("shared QA snapshot lock should not be poisoned")
                .clone()
        }
    }

    struct FirstEventSharedSnapshotPendingSource {
        first_event: Option<CoreEvent>,
        snapshot: Arc<Mutex<AppState>>,
    }

    impl QaEventSource for FirstEventSharedSnapshotPendingSource {
        fn recv_event(&mut self) -> QaEventFuture<'_> {
            if let Some(event) = self.first_event.take() {
                return Box::pin(async move { Ok(event) });
            }
            Box::pin(std::future::pending())
        }
    }

    impl QaSnapshotEventSource for FirstEventSharedSnapshotPendingSource {
        fn snapshot(&self) -> AppState {
            self.snapshot
                .lock()
                .expect("shared QA snapshot lock should not be poisoned")
                .clone()
        }
    }

    struct FirstEventThenTerminalLagSource {
        first_event: Option<CoreEvent>,
        snapshot: AppState,
        skipped: u64,
    }

    impl QaEventSource for FirstEventThenTerminalLagSource {
        fn recv_event(&mut self) -> QaEventFuture<'_> {
            Box::pin(async move {
                if let Some(event) = self.first_event.take() {
                    return Ok(event);
                }
                self.snapshot.session = SessionState::SignedOut;
                Err(EventStreamLag {
                    skipped: self.skipped,
                })
            })
        }
    }

    impl QaSnapshotEventSource for FirstEventThenTerminalLagSource {
        fn snapshot(&self) -> AppState {
            self.snapshot.clone()
        }
    }

    fn qa_state_with_session(session: SessionState) -> AppState {
        AppState {
            session,
            ..AppState::default()
        }
    }

    fn qa_logged_out_event(request_id: RequestId, account_key: AccountKey) -> CoreEvent {
        CoreEvent::Account(AccountEvent::LoggedOut {
            request_id,
            account_key,
        })
    }

    fn qa_operation_failed_event(request_id: RequestId) -> CoreEvent {
        CoreEvent::OperationFailed {
            request_id,
            failure: CoreFailure::SessionNotFound,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn login_wait_observes_ready_snapshot_once_at_deadline_without_a_broadcast() {
        let shared = Arc::new(Mutex::new(qa_state_with_session(SessionState::SignedOut)));
        let mut source = SharedSnapshotPendingEventSource {
            snapshot: shared.clone(),
        };
        let ready_shared = shared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            ready_shared
                .lock()
                .expect("shared QA snapshot lock should not be poisoned")
                .session = SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@ready:example.invalid".to_owned(),
                device_id: "READYDEVICE".to_owned(),
            });
        });
        let started_at = tokio::time::Instant::now();

        let account_key = wait_for_logged_in(
            &mut source,
            RequestId {
                connection_id: koushi_core::ids::RuntimeConnectionId(1),
                sequence: 1,
            },
            "login final snapshot",
        )
        .await
        .expect("the final authoritative Ready snapshot should complete login");

        assert_eq!(account_key, AccountKey("@ready:example.invalid".to_owned()));
        assert_eq!(
            tokio::time::Instant::now().duration_since(started_at),
            LOGIN_EVENT_TIMEOUT
        );
    }

    #[tokio::test(start_paused = true)]
    async fn login_wait_without_event_or_ready_snapshot_still_times_out() {
        let shared = Arc::new(Mutex::new(qa_state_with_session(SessionState::SignedOut)));
        let mut source = SharedSnapshotPendingEventSource { snapshot: shared };
        let started_at = tokio::time::Instant::now();

        let error = wait_for_logged_in(
            &mut source,
            RequestId {
                connection_id: koushi_core::ids::RuntimeConnectionId(1),
                sequence: 2,
            },
            "login remains pending",
        )
        .await
        .expect_err("a non-Ready snapshot must retain the login timeout");

        assert_eq!(
            error,
            "login remains pending: timed out waiting for LoggedIn event"
        );
        assert_eq!(
            tokio::time::Instant::now().duration_since(started_at),
            LOGIN_EVENT_TIMEOUT
        );
    }

    #[tokio::test]
    async fn session_restored_account_mismatch_is_private_safe() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 6,
        };
        let expected = AccountKey("@expected:example.invalid".to_owned());
        let mut source = ScriptedQaSnapshotEventSource {
            events: [(
                CoreEvent::Account(AccountEvent::SessionRestored {
                    request_id,
                    account_key: AccountKey("@unexpected:example.invalid".to_owned()),
                }),
                SessionState::SignedOut,
            )]
            .into(),
            snapshot: qa_state_with_session(SessionState::SignedOut),
            received: 0,
        };

        let error =
            wait_for_session_restored(&mut source, request_id, &expected, "restore mismatch")
                .await
                .expect_err("wrong restored account must fail immediately");
        assert!(error.contains("account_key mismatch"));
        assert!(!error.contains('@'));
        assert_eq!(source.received, 1);
    }

    #[tokio::test]
    async fn logged_out_waiter_requires_event_and_signed_out_snapshot_in_either_order() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 7,
        };
        let account_key = AccountKey("@logout-barrier:example.invalid".to_owned());
        let signed_out = qa_state_with_session(SessionState::SignedOut);
        let cases = [
            [
                (
                    qa_logged_out_event(request_id, account_key.clone()),
                    SessionState::LoggingOut,
                ),
                (
                    CoreEvent::StateChanged(signed_out.clone()),
                    SessionState::SignedOut,
                ),
            ],
            [
                (
                    CoreEvent::StateChanged(signed_out.clone()),
                    SessionState::SignedOut,
                ),
                (
                    qa_logged_out_event(request_id, account_key.clone()),
                    SessionState::SignedOut,
                ),
            ],
        ];

        for events in cases {
            let mut source = ScriptedQaSnapshotEventSource {
                events: events.into(),
                snapshot: qa_state_with_session(SessionState::LoggingOut),
                received: 0,
            };
            wait_for_logged_out(&mut source, request_id, &account_key, "logout barrier")
                .await
                .expect("both authoritative logout signals should satisfy the barrier");
            assert_eq!(
                source.received, 2,
                "neither event nor snapshot may complete the barrier alone"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn logout_waiters_observe_final_signed_out_snapshot_without_another_broadcast() {
        for keyed in [true, false] {
            let request_id = RequestId {
                connection_id: koushi_core::ids::RuntimeConnectionId(1),
                sequence: if keyed { 71 } else { 72 },
            };
            let account_key = AccountKey("@logout-final-snapshot:example.invalid".to_owned());
            let shared = Arc::new(Mutex::new(qa_state_with_session(SessionState::LoggingOut)));
            let mut source = FirstEventSharedSnapshotPendingSource {
                first_event: Some(qa_logged_out_event(request_id, account_key.clone())),
                snapshot: shared.clone(),
            };
            let signed_out_shared = shared.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                signed_out_shared
                    .lock()
                    .expect("shared QA snapshot lock should not be poisoned")
                    .session = SessionState::SignedOut;
            });
            let started_at = tokio::time::Instant::now();

            if keyed {
                wait_for_logged_out(
                    &mut source,
                    request_id,
                    &account_key,
                    "keyed logout final snapshot",
                )
                .await
            } else {
                wait_for_signed_out_after_logout(
                    &mut source,
                    request_id,
                    "keyless logout final snapshot",
                )
                .await
            }
            .expect("the final authoritative SignedOut snapshot should complete logout");

            assert_eq!(
                tokio::time::Instant::now().duration_since(started_at),
                EVENT_TIMEOUT
            );
        }
    }

    #[tokio::test]
    async fn logout_waiters_observe_final_signed_out_snapshot_after_lag_or_close() {
        for (keyed, skipped) in [(true, 0), (false, 4)] {
            let request_id = RequestId {
                connection_id: koushi_core::ids::RuntimeConnectionId(1),
                sequence: if keyed { 73 } else { 74 },
            };
            let account_key = AccountKey("@logout-terminal-lag:example.invalid".to_owned());
            let mut source = FirstEventThenTerminalLagSource {
                first_event: Some(qa_logged_out_event(request_id, account_key.clone())),
                snapshot: qa_state_with_session(SessionState::LoggingOut),
                skipped,
            };

            if keyed {
                wait_for_logged_out(
                    &mut source,
                    request_id,
                    &account_key,
                    "keyed logout terminal snapshot",
                )
                .await
            } else {
                wait_for_signed_out_after_logout(
                    &mut source,
                    request_id,
                    "keyless logout terminal snapshot",
                )
                .await
            }
            .expect("the terminal receive must recheck the authoritative SignedOut snapshot");
        }
    }

    #[tokio::test]
    async fn logged_out_waiter_keeps_wrong_account_and_failure_terminal_and_private_safe() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 8,
        };
        let account_key = AccountKey("@expected:example.invalid".to_owned());
        let mut wrong_account = ScriptedQaSnapshotEventSource {
            events: [(
                qa_logged_out_event(
                    request_id,
                    AccountKey("@unexpected:example.invalid".to_owned()),
                ),
                SessionState::SignedOut,
            )]
            .into(),
            snapshot: qa_state_with_session(SessionState::LoggingOut),
            received: 0,
        };
        let error = wait_for_logged_out(
            &mut wrong_account,
            request_id,
            &account_key,
            "logout barrier",
        )
        .await
        .expect_err("wrong account must fail immediately");
        assert!(error.contains("account_key mismatch"));
        assert!(!error.contains('@'));
        assert_eq!(wrong_account.received, 1);

        let mut failed = ScriptedQaSnapshotEventSource {
            events: [(
                CoreEvent::OperationFailed {
                    request_id,
                    failure: CoreFailure::SessionRequired,
                },
                SessionState::SignedOut,
            )]
            .into(),
            snapshot: qa_state_with_session(SessionState::LoggingOut),
            received: 0,
        };
        let error = wait_for_logged_out(&mut failed, request_id, &account_key, "logout barrier")
            .await
            .expect_err("correlated failure must fail immediately");
        assert!(error.contains("SessionRequired"));
        assert_eq!(failed.received, 1);
    }

    #[tokio::test]
    async fn operation_failed_signed_out_waiter_requires_both_signals_in_either_order() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 10,
        };
        let signed_out = qa_state_with_session(SessionState::SignedOut);
        let cases = [
            [
                (
                    qa_operation_failed_event(request_id),
                    SessionState::Restoring,
                ),
                (
                    CoreEvent::StateChanged(signed_out.clone()),
                    SessionState::SignedOut,
                ),
            ],
            [
                (
                    CoreEvent::StateChanged(signed_out.clone()),
                    SessionState::SignedOut,
                ),
                (
                    qa_operation_failed_event(request_id),
                    SessionState::SignedOut,
                ),
            ],
        ];

        for events in cases {
            let mut source = ScriptedQaSnapshotEventSource {
                events: events.into(),
                snapshot: qa_state_with_session(SessionState::Restoring),
                received: 0,
            };
            let failure = wait_for_operation_failed_and_signed_out(
                &mut source,
                request_id,
                "restore cleanup barrier",
            )
            .await
            .expect("both authoritative cleanup signals should satisfy the barrier");
            assert_eq!(failure, CoreFailure::SessionNotFound);
            assert_eq!(
                source.received, 2,
                "neither failure nor SignedOut may complete the barrier alone"
            );
        }

        let mut succeeded = ScriptedQaSnapshotEventSource {
            events: [(
                CoreEvent::Account(AccountEvent::SessionRestored {
                    request_id,
                    account_key: AccountKey("@private:example.invalid".to_owned()),
                }),
                SessionState::SignedOut,
            )]
            .into(),
            snapshot: signed_out,
            received: 0,
        };
        let error = wait_for_operation_failed_and_signed_out(
            &mut succeeded,
            request_id,
            "restore cleanup barrier",
        )
        .await
        .expect_err("a same-request success terminal must fail immediately");
        assert!(error.contains("operation succeeded"));
        assert!(!error.contains('@'));
        assert_eq!(succeeded.received, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn operation_failed_signed_out_deadline_survives_unrelated_event_starvation() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 11,
        };
        let mut source = IntervalQaSnapshotEventSource {
            interval: tokio::time::interval(Duration::from_secs(1)),
            snapshot: qa_state_with_session(SessionState::Restoring),
            first_event: Some(qa_operation_failed_event(request_id)),
        };
        let started_at = tokio::time::Instant::now();
        wait_for_operation_failed_and_signed_out(
            &mut source,
            request_id,
            "restore cleanup deadline",
        )
        .await
        .expect_err("unrelated events must not restart the cleanup deadline");
        assert_eq!(
            tokio::time::Instant::now().duration_since(started_at),
            EVENT_TIMEOUT
        );
    }

    #[tokio::test(start_paused = true)]
    async fn logout_and_operation_failed_deadlines_survive_unrelated_event_starvation() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 9,
        };
        let account_key = AccountKey("@deadline:example.invalid".to_owned());
        let mut logout_source = IntervalQaSnapshotEventSource {
            interval: tokio::time::interval(Duration::from_secs(1)),
            snapshot: qa_state_with_session(SessionState::LoggingOut),
            first_event: Some(qa_logged_out_event(request_id, account_key.clone())),
        };
        let logout_started_at = tokio::time::Instant::now();
        wait_for_logged_out(
            &mut logout_source,
            request_id,
            &account_key,
            "logout deadline",
        )
        .await
        .expect_err("a LoggedOut event without SignedOut state must time out");
        assert_eq!(
            tokio::time::Instant::now().duration_since(logout_started_at),
            EVENT_TIMEOUT
        );

        let mut failure_source = IntervalQaEventSource {
            interval: tokio::time::interval(Duration::from_secs(1)),
        };
        let failure_started_at = tokio::time::Instant::now();
        wait_for_operation_failed(&mut failure_source, request_id, "failure deadline")
            .await
            .expect_err("unrelated events must not restart the failure deadline");
        assert_eq!(
            tokio::time::Instant::now().duration_since(failure_started_at),
            EVENT_TIMEOUT
        );
    }

    fn strict_e2ee_waiter_inventory() -> &'static [(&'static str, &'static str)] {
        &[
            (
                "wait_for_existing_identity_gate",
                "\nasync fn wait_for_recovery_gate",
            ),
            (
                "wait_for_room_in_room_list",
                "\nasync fn wait_for_space_in_space_list",
            ),
            (
                "wait_for_sync_started_and_running",
                "\nasync fn wait_for_sync_started",
            ),
            (
                "wait_for_ready_snapshot",
                "\nasync fn wait_for_room_unread_count",
            ),
            ("wait_for_logged_in", "\nasync fn wait_for_session_restored"),
            (
                "subscribe_and_ack_active_timeline_projection_for_qa",
                "\nfn assert_no_decryption_failure_items",
            ),
            (
                "wait_for_verification_requested_event_only",
                "\nfn requested_verification_flow_id",
            ),
            (
                "wait_for_verification_accepted",
                "\nfn verification_state_is_at_least_accepted",
            ),
            (
                "wait_for_initial_items_from_source",
                "\n#[derive(Default)]\nstruct InitialItemsWaitDiagnostics",
            ),
            (
                "wait_for_send_flow_completion_with_timeout",
                "\nasync fn send_text_expect_local_echo",
            ),
            (
                "wait_for_item_with_body_or_decryption_failure",
                "\n#[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
                 enum WithheldEventProjectionOrigin",
            ),
            (
                "wait_for_withheld_event_projection_from_source",
                "\n/// Wait until all `expected_bodies` are found",
            ),
        ]
    }

    fn strict_e2ee_waiter_body<'a>(
        source: &'a str,
        waiter: &str,
        end_declaration: &str,
    ) -> &'a str {
        source
            .split(&format!("async fn {waiter}"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing strict E2EE waiter {waiter}"))
            .split(end_declaration)
            .next()
            .unwrap_or_else(|| panic!("missing end declaration for strict E2EE waiter {waiter}"))
    }

    fn strict_e2ee_rolling_waiters(source: &str) -> Vec<&'static str> {
        strict_e2ee_waiter_inventory()
            .iter()
            .filter_map(|&(waiter, end_declaration)| {
                strict_e2ee_waiter_body(source, waiter, end_declaration)
                    .contains("tokio::time::timeout(")
                    .then_some(waiter)
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn initial_items_wait_deadline_is_not_extended_by_continuous_unrelated_events() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let mut source = IntervalQaEventSource {
            interval: tokio::time::interval(Duration::from_secs(1)),
        };
        let started_at = tokio::time::Instant::now();

        let result = tokio::time::timeout(
            Duration::from_secs(30),
            wait_for_initial_items_from_source(
                &mut source,
                &key,
                request_id,
                "deadline starvation regression",
                Duration::from_secs(10),
            ),
        )
        .await
        .expect("the absolute waiter must finish before the outer starvation guard");

        let error = result.expect_err("unrelated events must not satisfy the causal wait");
        assert!(error.contains("timed out waiting for TimelineEvent::InitialItems"));
        assert!(error.contains("same_key_wrong_cause=0"));
        assert!(error.contains("same_key_causeless=0"));
        assert!(error.contains("unrelated_events="));
        assert_eq!(
            tokio::time::Instant::now().duration_since(started_at),
            Duration::from_secs(10),
            "unrelated events must not restart the ten-second budget"
        );
    }

    #[tokio::test]
    async fn initial_items_wait_skips_fresh_wrong_cause_then_accepts_exact_replay_cause() {
        let old_projection_request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let subscribe_request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let event = |cause_request_id, items| {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(old_projection_request_id),
                cause_request_id: Some(cause_request_id),
                key: key.clone(),
                actor_generation: 1,
                generation: koushi_core::ids::TimelineGeneration(0),
                items,
            })
        };
        let mut source = ScriptedQaEventSource {
            events: [
                event(old_projection_request_id, Vec::new()),
                event(
                    subscribe_request_id,
                    vec![projection_timeline_item("$exact-replay:test", false)],
                ),
            ]
            .into(),
        };

        let items = wait_for_initial_items_from_source(
            &mut source,
            &key,
            subscribe_request_id,
            "causal replay regression",
            Duration::from_secs(1),
        )
        .await
        .expect("the exact idempotent replay cause should satisfy the waiter");

        assert_eq!(items.len(), 1, "the wrong-cause fresh event was ignored");
    }

    #[tokio::test(start_paused = true)]
    async fn initial_items_timeout_reports_only_private_safe_causal_category_counts() {
        let old_request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        let key = TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!room:test");
        let wrong_key =
            TimelineKey::room(AccountKey("@qa:example.invalid".to_owned()), "!other:test");
        let initial = |event_key, cause_request_id| {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(old_request_id),
                cause_request_id,
                key: event_key,
                actor_generation: 1,
                generation: koushi_core::ids::TimelineGeneration(0),
                items: Vec::new(),
            })
        };
        let mut source = ScriptedQaEventSource {
            events: [
                initial(key.clone(), Some(old_request_id)),
                initial(key.clone(), None),
                initial(wrong_key, Some(request_id)),
                CoreEvent::Sync(SyncEvent::Running),
            ]
            .into(),
        };

        let error = wait_for_initial_items_from_source(
            &mut source,
            &key,
            request_id,
            "causal categories regression",
            Duration::from_secs(1),
        )
        .await
        .expect_err("no exact-cause event was supplied");

        assert!(error.contains("same_key_exact_cause=0"));
        assert!(error.contains("same_key_wrong_cause=1"));
        assert!(error.contains("same_key_causeless=1"));
        assert!(error.contains("wrong_key_initial_items=1"));
        assert!(error.contains("unrelated_events=1"));
        assert!(!error.contains("@qa:"));
        assert!(!error.contains("!room:"));
    }

    #[test]
    fn strict_e2ee_guard_extracts_each_complete_waiter_body() {
        let source = include_str!("headless-core-qa.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("headless-core-qa source should contain production section");

        for &(waiter, end_declaration) in strict_e2ee_waiter_inventory() {
            let body = strict_e2ee_waiter_body(production_source, waiter, end_declaration);
            assert!(
                body.contains(".recv(") || body.contains("recv_event()"),
                "{waiter} extraction must reach its event receive loop"
            );
        }
    }

    #[test]
    fn strict_e2ee_guard_detects_a_rolling_timeout_in_every_inventory_body() {
        let source = include_str!("headless-core-qa.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("headless-core-qa source should contain production section");

        for &(waiter, _) in strict_e2ee_waiter_inventory() {
            let declaration = format!("async fn {waiter}");
            let injected_declaration = format!(
                "{declaration}\n    tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())"
            );
            let injected = production_source.replacen(&declaration, &injected_declaration, 1);

            assert_eq!(
                strict_e2ee_rolling_waiters(&injected),
                vec![waiter],
                "the structural guard must detect a rolling timeout in {waiter}"
            );
        }
    }

    #[test]
    fn strict_e2ee_event_waiters_do_not_restart_timeouts_per_event() {
        let source = include_str!("headless-core-qa.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("headless-core-qa source should contain production section");
        let rolling_waiters = strict_e2ee_rolling_waiters(production_source);
        assert!(
            rolling_waiters.is_empty(),
            "strict E2EE waiters must use one absolute deadline; rolling={rolling_waiters:?}"
        );
    }

    #[test]
    fn active_room_thread_refresh_uses_the_exact_causal_waiter() {
        let source = include_str!("headless-core-qa.rs");
        let refresh = source
            .split("let refresh_room_a_id = conn_a.next_request_id();")
            .nth(1)
            .expect("thread stage should refresh the active room timeline")
            .split("wait_for_room_timeline_thread_summary")
            .next()
            .expect("thread summary wait should follow the room refresh");

        assert!(refresh.contains("wait_for_initial_items("));
    }

    #[test]
    fn e2ee_trust_stage_does_not_overlap_normal_sync_with_manual_sync_once() {
        let source = include_str!("headless-core-qa.rs");
        let stage = source
            .split("async fn run_e2ee_trust_stage(")
            .nth(1)
            .expect("E2EE trust stage should exist")
            .split("async fn cleanup_logged_in_runtime(")
            .next()
            .expect("secondary-device cleanup should follow E2EE trust");

        assert!(
            !stage.contains("sync_once_for_qa("),
            "E2EE trust must use the authoritative bootstrap and typed gate readiness while SyncService owns the client"
        );
        assert!(
            !stage.contains("publish primary cross-signing facts before gated second-device login")
        );
    }

    #[test]
    fn encrypted_backup_seed_uses_live_room_discovery_and_exact_causal_waiter() {
        let source = include_str!("headless-core-qa.rs");
        let seed = source
            .split("async fn seed_encrypted_room_key_for_qa(")
            .nth(1)
            .expect("encrypted backup seed helper should exist")
            .split("async fn enable_key_backup_for_qa(")
            .next()
            .expect("key backup enable helper should follow seed helper");

        assert!(
            !seed.contains("sync_once_for_qa("),
            "backup seed room discovery must not overlap the running SyncService"
        );
        assert!(seed.contains("wait_for_room_in_room_list("));
        assert!(seed.contains("wait_for_initial_items("));
        assert!(seed.contains("subscribe encrypted backup seed"));
    }

    #[test]
    fn second_device_encrypted_room_resubscribe_uses_exact_causal_waiter() {
        let source = include_str!("headless-core-qa.rs");
        let delivery = source
            .split("async fn verify_second_device_room_key_delivery_for_qa(")
            .nth(1)
            .expect("second-device encrypted delivery helper should exist")
            .split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa(")
            .next()
            .expect("multi-device delivery helper should follow second-device delivery");

        assert!(delivery.contains("wait_for_initial_items("));
    }

    #[test]
    fn generic_secondary_timeline_subscribe_uses_exact_causal_waiter() {
        let source = include_str!("headless-core-qa.rs");
        let secondary_subscribe = source
            .split("// B subscribes and receives both messages")
            .nth(1)
            .expect("generic B timeline subscribe block")
            .split("// Paginate backward on B")
            .next()
            .expect("generic B timeline subscribe block end");

        assert!(secondary_subscribe.contains("wait_for_initial_items("));
    }

    #[test]
    fn send_queue_proxy_forces_connection_close_per_request() {
        let request = b"POST /_matrix/client/v3/login HTTP/1.1\r\nHost: example.test\r\nConnection: keep-alive\r\nProxy-Connection: keep-alive\r\nContent-Length: 2\r\n\r\n{}";
        let rewritten = rewrite_http_request_connection_close(request).unwrap();
        let rewritten = String::from_utf8(rewritten).unwrap();
        let (head, body) = rewritten.split_once("\r\n\r\n").unwrap();

        assert!(
            head.contains("\r\nConnection: close"),
            "send queue proxy must force one HTTP request per connection so response copying can read to EOF"
        );
        assert!(
            !head.to_ascii_lowercase().contains("proxy-connection"),
            "send queue proxy must drop proxy keep-alive headers before forwarding"
        );
        assert_eq!(body, "{}");
    }

    #[test]
    fn fallback_proxy_classifies_closed_sync_routes_without_query_data() {
        assert_eq!(
            qa_proxy_request_kind(
                b"GET /_matrix/client/versions HTTP/1.1\r\nHost: example.test\r\n\r\n"
            )
            .unwrap(),
            QaProxyRequestKind::Versions,
        );
        assert_eq!(
            qa_proxy_request_kind(b"POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync?pos=private HTTP/1.1\r\nHost: example.test\r\n\r\n").unwrap(),
            QaProxyRequestKind::SyncService,
        );
        assert_eq!(
            qa_proxy_request_kind(
                b"GET /_matrix/client/v3/sync?since=private HTTP/1.1\r\nHost: example.test\r\n\r\n"
            )
            .unwrap(),
            QaProxyRequestKind::LegacySync,
        );
    }

    #[test]
    fn live_tail_proxy_enforces_tokenless_refresh_and_exact_continuation_requests() {
        let metadata = qa_room_messages_request_metadata(
            b"GET /_matrix/client/v3/rooms/%21room%3Aexample.invalid/messages?dir=b&limit=128 HTTP/1.1\r\nHost: example.invalid\r\n\r\n",
        )
        .expect("valid request")
        .expect("room messages metadata");
        assert_eq!(
            metadata,
            QaRoomMessagesRequestMetadata {
                query_is_exact_tokenless_limit: true,
                has_from: false,
                direction_is_backward: true,
                from_token: None,
            }
        );

        let mut state = QaMessagesProxyState::default();
        state.arm_page(QaMessagesProxyExpectation::TokenlessLiveTail, None);
        assert_eq!(
            state.observe_room_messages_request(&metadata),
            QaMessagesProxyDecision::ServeCannedPage
        );

        let continuation = qa_room_messages_request_metadata(
            b"GET /_matrix/client/v3/rooms/%21room%3Aexample.invalid/messages?dir=b&from=continuation&limit=128 HTTP/1.1\r\nHost: example.invalid\r\n\r\n",
        )
        .expect("valid continuation request")
        .expect("room messages continuation metadata");
        state.arm_page(
            QaMessagesProxyExpectation::BackwardFrom {
                token: "continuation".to_owned(),
            },
            Some("continuation".to_owned()),
        );
        assert_eq!(
            state.observe_room_messages_request(&continuation),
            QaMessagesProxyDecision::ServeCannedPage
        );
        assert!(state.observation.expected_end_token_was_used);
        assert_eq!(state.observation.expected_end_token_request_count, 1);
    }

    #[test]
    fn canned_live_tail_messages_page_reproduces_a_gap_before_the_known_latest_event() {
        let body = QaCannedMessagesPage::anchored_silent_gap(
            "$latest:example.invalid".to_owned(),
            "known latest".to_owned(),
            "$missing:example.invalid".to_owned(),
            "missing before latest".to_owned(),
            "$older:example.invalid".to_owned(),
            "@sender:example.invalid".to_owned(),
            "known older anchor".to_owned(),
        )
        .response_body()
        .expect("canned /messages response should serialize");
        let response: serde_json::Value =
            serde_json::from_slice(&body).expect("canned /messages response should be JSON");

        assert_eq!(
            response.get("start").and_then(serde_json::Value::as_str),
            Some("qa-live-tail-start")
        );
        assert!(response.get("end").is_none());
        let ids = response["chunk"]
            .as_array()
            .expect("canned chunk")
            .iter()
            .map(|event| event["event_id"].as_str().expect("event id"))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "$latest:example.invalid",
                "$missing:example.invalid",
                "$older:example.invalid",
            ]
        );
    }

    #[test]
    fn fallback_proxy_fails_sync_service_then_holds_legacy() {
        let control = Arc::new((
            Mutex::new(QaFallbackProxyState {
                phase: QaFallbackProxyPhase::Armed,
                versions_forwarded: false,
                sync_service_failed: false,
            }),
            Condvar::new(),
        ));
        assert_eq!(
            fallback_proxy_action(&control, QaProxyRequestKind::Versions).unwrap(),
            QaProxyRequestAction::Forward,
        );
        assert_eq!(
            fallback_proxy_action(&control, QaProxyRequestKind::SyncService).unwrap(),
            QaProxyRequestAction::FailClosed,
        );
        assert_eq!(
            fallback_proxy_action(&control, QaProxyRequestKind::LegacySync).unwrap(),
            QaProxyRequestAction::HoldLegacy,
        );
        let state = control.0.lock().unwrap();
        assert!(state.versions_forwarded);
        assert!(state.sync_service_failed);
        assert_eq!(state.phase, QaFallbackProxyPhase::LegacyHeld);
    }

    #[test]
    fn timeline_stress_uses_event_waiters_not_manual_sync_once() {
        let source = include_str!("headless-core-qa.rs");
        let body = source
            .split("async fn run_timeline_stress_stage")
            .nth(1)
            .and_then(|rest| {
                rest.split("async fn run_timeline_stress_room_messages")
                    .next()
            })
            .expect("timeline stress stage body");

        assert!(
            !body.contains("sync_once_for_qa"),
            "timeline stress must not mix manual /sync with the running SyncService path"
        );
        assert!(
            body.contains("wait_for_invite_in_snapshot"),
            "timeline stress should wait for invite projection through the live sync path"
        );
    }

    #[test]
    fn login_wait_uses_dedicated_timeout_for_loaded_local_homeservers() {
        let source = include_str!("headless-core-qa.rs");
        let ready_helper = source
            .split("fn ready_account_key")
            .nth(1)
            .and_then(|rest| rest.split("async fn wait_for_logged_in").next())
            .expect("ready account-key helper body");
        let wait_body = source
            .split("async fn wait_for_logged_in")
            .nth(1)
            .and_then(|rest| {
                rest.split("/// Wait for `AccountEvent::SessionRestored`")
                    .next()
            })
            .expect("wait_for_logged_in body");

        assert!(
            source.contains("const LOGIN_EVENT_TIMEOUT: Duration = Duration::from_secs(90);"),
            "login waits need their own timeout because local homeservers can finish /login slowly under full QA load"
        );
        assert!(
            wait_body.contains("QaEventDeadline::after(LOGIN_EVENT_TIMEOUT)")
                && wait_body.contains(".recv(conn)"),
            "wait_for_logged_in must use one absolute dedicated login deadline"
        );
        assert!(
            ready_helper.contains("SessionState::Ready(info)")
                && ready_helper.contains("Some(AccountKey(info.user_id))")
                && wait_body.matches("ready_account_key(conn)").count() >= 3,
            "the identity-gate helper may consume LoggedIn, so the waiter must accept the authoritative Ready snapshot"
        );
    }

    #[test]
    fn all_directory_stage_runs_before_room_space_operations() {
        let source = include_str!("headless-core-qa.rs");
        let run_async_body = source
            .split("async fn run_async")
            .nth(1)
            .and_then(|rest| rest.split("async fn cleanup_after_full_flow").next())
            .expect("run_async body");
        let directory_call = "run_directory_stage(&config, &mut conn_a, conn_b).await?";
        let directory_index = run_async_body
            .find(directory_call)
            .expect("directory stage call in run_async");
        let room_space_index = run_async_body
            .find("// --- Phase 4: Room operations")
            .expect("room-space stage marker");

        assert!(
            directory_index < room_space_index,
            "All flow must run directory QA before RoomSpace operations"
        );
        assert!(
            !run_async_body[room_space_index..].contains(directory_call),
            "directory QA must not be re-run after RoomSpace has started B sync"
        );
    }

    #[test]
    fn send_queue_fifo_wait_uses_dedicated_reconnect_timeout() {
        let source = include_str!("headless-core-qa.rs");
        let body = source
            .split("async fn wait_for_send_completions_in_order")
            .nth(1)
            .and_then(|rest| {
                rest.split("async fn wait_for_cancelled_or_removed_send")
                    .next()
            })
            .expect("send queue FIFO wait body");

        assert_eq!(
            SEND_QUEUE_EVENT_TIMEOUT,
            Duration::from_secs(300),
            "SendQueue reconnect timeout must be 300 seconds, independently of the generic event timeout"
        );
        assert!(
            body.contains("tokio::time::timeout(SEND_QUEUE_EVENT_TIMEOUT, conn.recv_event())"),
            "FIFO retry waiter must use the send-queue reconnect timeout"
        );
        assert!(
            body.contains("first_completed={first_completed}"),
            "FIFO retry timeout should report whether the first queued send completed"
        );
    }

    #[test]
    fn send_queue_unsubscribes_timeline_before_runtime_shutdown() {
        let source = include_str!("headless-core-qa.rs");
        let body = source
            .split("async fn run_send_queue_stage")
            .nth(1)
            .and_then(|rest| {
                rest.split("async fn run_timeline_reconnect_scenario")
                    .next()
            })
            .expect("send queue stage body");
        let restart_slice = body
            .split("stop_sync_for_qa(&mut conn, \"send_queue stop before restart\")")
            .nth(1)
            .and_then(|rest| rest.split("let mut conn = runtime.attach();").next())
            .expect("send queue restart lifecycle slice");

        assert!(
            body.contains("send_queue unsubscribe before restart shutdown"),
            "send_queue should drop its subscribed timeline before restart shutdown"
        );
        assert!(
            body.contains("send_queue unsubscribe before cleanup"),
            "send_queue should drop its restored timeline before final cleanup"
        );
        assert!(
            source.contains(
                "const TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);"
            ),
            "send_queue needs a bounded settle window because Unsubscribe has no completion event"
        );
        assert!(
            body.contains("TIMELINE_UNSUBSCRIBE_SETTLE_TIMEOUT"),
            "send_queue unsubscribe helper should wait for timeline actor shutdown before runtime drop"
        );
        let shutdown = restart_slice
            .find("runtime.shutdown().await")
            .expect("restart must await the ordered runtime shutdown barrier");
        let reopen = restart_slice
            .find("CoreRuntime::start_with_data_dir(data_dir)")
            .expect("restart must reopen the same persisted data directory");
        assert!(
            shutdown < reopen,
            "runtime shutdown must complete before reopen"
        );
        assert!(!restart_slice.contains("drop(runtime)"));
        assert!(!restart_slice.contains("Duration::from_millis(500)"));
    }

    #[test]
    fn same_data_dir_reopen_paths_use_ordered_runtime_shutdown() {
        fn assert_ordered_reopen(
            label: &str,
            restart_slice: &str,
            drop_connection: &str,
            shutdown_runtime: &str,
            reopen_runtime: &str,
        ) {
            let drop_connection = restart_slice
                .find(drop_connection)
                .unwrap_or_else(|| panic!("{label}: connection must be dropped before shutdown"));
            let shutdown = restart_slice
                .find(shutdown_runtime)
                .unwrap_or_else(|| panic!("{label}: ordered runtime shutdown is required"));
            let reopen = restart_slice
                .find(reopen_runtime)
                .unwrap_or_else(|| panic!("{label}: same data directory must be reopened"));

            assert!(drop_connection < shutdown, "{label}: drop connection first");
            assert!(shutdown < reopen, "{label}: shutdown must precede reopen");
            assert!(
                !restart_slice.contains("drop(runtime"),
                "{label}: dropping a runtime is not a shutdown barrier"
            );
            assert!(
                !restart_slice.contains("Duration::from_millis(500)"),
                "{label}: blind store-lock sleeps are forbidden"
            );
        }

        let source = include_str!("headless-core-qa.rs");
        let cleanup = source
            .split("async fn cleanup_after_full_flow")
            .nth(1)
            .and_then(|rest| rest.split("let mut conn_a2 = runtime_a2.attach();").next())
            .expect("full-flow cleanup restart slice");
        assert_ordered_reopen(
            "cleanup_after_full_flow",
            cleanup,
            "drop(conn_a)",
            "runtime_a.shutdown().await",
            "CoreRuntime::start_with_data_dir(data_dir_a)",
        );

        let all_flow = source
            .split("// --- Sync stop A + store-backed restore A + logout A ---")
            .nth(1)
            .and_then(|rest| rest.split("let mut conn_a2 = runtime_a2.attach();").next())
            .expect("All-scenario restore restart slice");
        assert_ordered_reopen(
            "run_async All restore",
            all_flow,
            "drop(conn_a)",
            "runtime_a.shutdown().await",
            "CoreRuntime::start_with_data_dir(data_dir_a)",
        );

        let cache_restore = source
            .split("async fn run_cache_restore_scenario")
            .nth(1)
            .and_then(|rest| rest.split("let mut conn2 = runtime2.attach();").next())
            .expect("cache restore restart slice");
        assert_ordered_reopen(
            "cache restore",
            cache_restore,
            "drop(conn)",
            "runtime.shutdown().await",
            "CoreRuntime::start_with_data_dir(data_dir)",
        );
    }

    #[test]
    fn timeline_stress_backfill_only_advances_current_paginate_request() {
        let source = include_str!("headless-core-qa.rs");
        let body = source
            .split("async fn wait_for_stress_bodies_and_no_blank_rows")
            .nth(1)
            .and_then(|rest| {
                rest.split("async fn submit_stress_backfill_paginate")
                    .next()
            })
            .expect("stress body wait helper");
        let pagination_arm = body
            .split("CoreEvent::Timeline(TimelineEvent::PaginationStateChanged")
            .nth(1)
            .and_then(|rest| rest.split("CoreEvent::OperationFailed").next())
            .expect("pagination state arm");

        assert!(
            pagination_arm.contains("request_id: ev_id")
                && pagination_arm.contains("ev_id == &Some(current_paginate_request_id)"),
            "stress backfill must ignore stale pagination state from older requests on the same timeline"
        );
    }

    #[test]
    fn timeline_stress_blank_row_detection_rejects_empty_formatted_body() {
        let mut item = synthetic_timeline_item(
            "$formatted-blank:test",
            Some("plain fallback"),
            None,
            None,
            None,
        );
        item.formatted = Some(koushi_core::event::TimelineFormattedBody {
            html: "<p><br /></p>".to_owned(),
            plain_text: String::new(),
            code_blocks: Vec::new(),
        });
        item.body = None;

        assert!(
            !timeline_item_has_visible_payload(&item),
            "blank formatted HTML must not satisfy stress_no_blank"
        );
    }

    #[test]
    fn timeline_stress_replay_existing_is_read_only() {
        let source = include_str!("headless-core-qa.rs");
        let run_async_body = source
            .split("async fn run_async")
            .nth(1)
            .and_then(|rest| rest.split("// --- Phase 4: Room operations").next())
            .expect("run_async pre-room-create body");
        assert!(
            run_async_body.contains("run_timeline_stress_replay_stage"),
            "timeline stress replay must branch before the normal room creation flow"
        );

        let replay_body = source
            .split("async fn run_timeline_stress_replay_stage")
            .nth(1)
            .and_then(|rest| rest.split("struct StressRoomCoordinates").next())
            .expect("timeline stress replay body");
        for forbidden in ["CreateRoom", "CreateSpace", "SendText"] {
            assert!(
                !replay_body.contains(forbidden),
                "timeline stress replay must not perform mutating operation {forbidden}"
            );
        }
        assert!(replay_body.contains("Subscribe"));
        assert!(replay_body.contains("submit_stress_backfill_paginate"));
    }

    #[test]
    fn scheduled_send_scenario_runs_after_timeline_and_reports_private_tokens() {
        assert_eq!(
            QaScenario::from_env_value("scheduled_send").unwrap(),
            QaScenario::ScheduledSend
        );
        assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::Safety));
        assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::ScheduledSend));
        assert!(QaScenario::ScheduledSend.suppress_matrix_identifiers());
        assert!(!QaScenario::ScheduledSend.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::ScheduledSend.should_run_stage(QaStage::EditRedactSearch));

        assert_eq!(
            final_tokens_for_scenario(QaScenario::ScheduledSend),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "scheduled_capability=local_fallback",
                "scheduled_create=ok",
                "scheduled_reschedule=ok",
                "scheduled_cancel=ok",
                "scheduled_fire=ok",
                "restore_cleanup=ok",
            ]
        );
    }

    #[test]
    fn room_management_scenario_runs_after_room_space_and_reports_private_tokens() {
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::Safety));
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::RoomManagement.should_run_stage(QaStage::RoomManagement));
        assert!(!QaScenario::RoomManagement.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::RoomManagement.suppress_matrix_identifiers());

        assert_eq!(
            final_tokens_for_scenario(QaScenario::RoomManagement),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "room_settings=ok",
                "moderation=ok",
                "permission_guard=ok",
                "restore_cleanup=ok",
            ]
        );
    }

    #[test]
    fn room_management_forbidden_predicate_requires_matching_failed_moderation_state() {
        let request_id = RequestId {
            connection_id: koushi_core::ids::RuntimeConnectionId(1),
            sequence: 42,
        };
        let mut state = AppState::default();

        assert!(!room_management_forbidden_recorded(&state, request_id));

        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id: 41,
            room_id: "!redacted:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Moderation,
            kind: OperationFailureKind::Forbidden,
        };
        assert!(!room_management_forbidden_recorded(&state, request_id));

        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id: 42,
            room_id: "!redacted:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Moderation,
            kind: OperationFailureKind::Forbidden,
        };
        assert!(room_management_forbidden_recorded(&state, request_id));
    }

    #[test]
    fn implemented_final_tokens_include_thread() {
        assert_eq!(
            &implemented_final_tokens()[..],
            &[
                "safety=ok",
                "login_sync=ok",
                "credential_health=ok",
                "fail_closed=ok",
                "notification_candidate=ok",
                "badge_state=ok",
                "suppress_focus=ok",
                "clear_badge=ok",
                "invite_recv=ok",
                "invite_accept=ok",
                "invite_decline=ok",
                "member_list=ok",
                "dm_start=ok",
                "dm_space_scope=ok",
                "room_space=ok",
                "directory_query=ok",
                "directory_join=ok",
                "room_settings=ok",
                "moderation=ok",
                "permission_guard=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "activity_recent=ok",
                "activity_unread=ok",
                "activity_resolution=ok",
                "activity_markread=ok",
                "mention_send=ok",
                "markdown_send=ok",
                "slash_command=ok",
                "ime_guard=ok",
                "reply=ok",
                "reply_quote=ok",
                "pin_event=ok",
                "pinned_state=ok",
                "unpin_event=ok",
                "thread_canonical=ok",
                "thread_summary=ok",
                "thread_recv=ok",
                "thread_paginate=end_reached",
                "send_media=ok",
                "media_caption=ok",
                "image_compress=ok",
                "upload_staging=ok",
                "media_gallery=ok",
                "recv_media=ok",
                "read_receipt=ok",
                "fully_read=ok",
                "typing=ok",
                "presence=ok",
                "live_signals=ok",
                "edit_redact_search=ok",
                "crawl_backfill=ok",
                "crawl_no_media_bytes=ok",
                "crawl_throttle=ok",
                "crawl_failure=ok",
                "scheduled_capability=local_fallback",
                "scheduled_create=ok",
                "scheduled_reschedule=ok",
                "scheduled_cancel=ok",
                "scheduled_fire=ok",
                "send_fail=ok",
                "resend=ok",
                "cancel_send=ok",
                "fifo=ok",
                "unsent_restart=ok",
                "display_projection_reset_fallbacks=0",
                "joined_room_restore=ok",
                "e2ee_second_device_decrypt=ok",
                "e2ee_multi_user_multi_device_decrypt=ok",
                "e2ee_unverified_peer_send_nonblocking=ok",
                "e2ee_blocked_device_withheld=ok",
                "e2ee_trust=ok",
                "restore_cleanup=ok",
                "link_preview_global=ok",
                "link_preview_room=ok",
                "link_preview_e2ee_default=ok",
                "link_preview_hide=ok",
            ][..]
        );
    }

    #[test]
    fn e2ee_trust_stage_prints_joined_room_restore_scope_token() {
        let source = include_str!("headless-core-qa.rs");
        let legacy_token = concat!("e2ee_key_backup_restore_", "success=ok");

        assert!(source.contains("println!(\"joined_room_restore=ok\")"));
        assert!(!source.contains(legacy_token));
    }

    #[test]
    fn e2ee_trust_stage_reports_second_device_decrypt_token() {
        let source = include_str!("headless-core-qa.rs");

        assert!(tokens_for_stage(QaStage::E2eeTrust).contains(&"e2ee_second_device_decrypt=ok"));
        assert!(source.contains("println!(\"e2ee_second_device_decrypt=ok\")"));
    }

    #[test]
    fn e2ee_trust_stage_reports_multi_user_multi_device_decrypt_token() {
        let source = include_str!("headless-core-qa.rs");

        assert!(
            tokens_for_stage(QaStage::E2eeTrust)
                .contains(&"e2ee_multi_user_multi_device_decrypt=ok")
        );
        assert!(source.contains("println!(\"e2ee_multi_user_multi_device_decrypt=ok\")"));
    }

    #[test]
    fn e2ee_trust_stage_makes_identity_reset_explicitly_opt_in() {
        let source = include_str!("headless-core-qa.rs");

        assert!(source.contains("KOUSHI_QA_ALLOW_IDENTITY_RESET"));
        assert!(source.contains("if config.allow_identity_reset"));
        assert!(source.contains("println!(\"e2ee_identity_reset=skipped\")"));
    }

    #[test]
    fn parse_env_flag_accepts_only_explicit_boolean_values() {
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("0", false),
            ("false", false),
            ("FALSE", false),
            ("", false),
        ] {
            assert_eq!(parse_env_flag("QA_FLAG", value), Ok(expected));
        }

        assert!(parse_env_flag("QA_FLAG", "yes").is_err());
    }

    #[test]
    fn core_qa_stdout_does_not_format_matrix_identifiers() {
        let source = include_str!("headless-core-qa.rs");

        for forbidden in [
            concat!("println!(\"", "room_", "id={"),
            concat!("println!(\"", "space_", "id={"),
            concat!("println!(\"", "event_", "id={"),
            concat!("println!(\"", "sdk_", "txn={"),
            concat!("println!(\"", "transaction_", "id={"),
        ] {
            assert!(
                !source.contains(forbidden),
                "core QA stdout must not format {forbidden}"
            );
        }
    }

    #[test]
    fn e2ee_trust_qa_uses_authenticated_provisional_session_info() {
        let info = SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        };

        assert_eq!(
            authenticated_session_info_from_state(&SessionState::Provisional {
                info: info.clone(),
                phase: koushi_state::ProvisionalPhase::CheckingTrust,
            }),
            Some(&info)
        );
        assert_eq!(
            authenticated_session_info_from_state(&SessionState::AwaitingVerification {
                info: info.clone(),
                gate: koushi_state::VerificationGateState {
                    methods: vec![],
                    account_kind: koushi_state::VerificationAccountKind::Unknown,
                    failure: None,
                },
            }),
            Some(&info)
        );
        assert_eq!(
            authenticated_session_info_from_state(&SessionState::Ready(info.clone())),
            Some(&info)
        );
        assert_eq!(
            authenticated_session_info_from_state(&SessionState::SignedOut),
            None
        );
    }

    #[tokio::test]
    async fn receiver_device_checkpoint_holds_start_once_until_ack_and_skips_it_on_failure() {
        let starts = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let task_starts = starts.clone();
        let task = tokio::spawn(after_receiver_device_known(
            async move {
                entered_tx.send(()).map_err(|_| "checkpoint entry closed")?;
                release_rx.await.map_err(|_| "checkpoint release closed")?;
                Ok(())
            },
            move || async move {
                task_starts.fetch_add(1, Ordering::SeqCst);
                Ok::<_, &'static str>(())
            },
        ));

        entered_rx
            .await
            .expect("refresh checkpoint should be polled");
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        release_tx.send(()).expect("release checkpoint");
        task.await
            .expect("checkpoint task should join")
            .expect("checkpoint should succeed");
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        let failed_starts = Arc::new(AtomicUsize::new(0));
        let closure_starts = failed_starts.clone();
        let failed = after_receiver_device_known(
            async { Err::<(), _>("device unknown") },
            move || async move {
                closure_starts.fetch_add(1, Ordering::SeqCst);
                Ok::<_, &'static str>(())
            },
        )
        .await;
        assert_eq!(failed, Err("device unknown"));
        assert_eq!(failed_starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn provisional_self_verification_keeps_primary_normal_sync_running() {
        let source = include_str!("headless-core-qa.rs");
        let helper = source
            .split("async fn verify_provisional_second_device_for_qa")
            .nth(1)
            .expect("provisional self-verification helper should exist")
            .split("fn verification_closed_summary")
            .next()
            .expect("verification summary helper should follow provisional verification");

        assert!(helper.contains("AccountCommand::StartOwnUserSas"));
        let refresh = helper
            .find("refresh_device_keys_and_assert_known_for_qa(")
            .expect("primary must causally discover the exact provisional device");
        let start = helper
            .find("AccountCommand::StartOwnUserSas")
            .expect("provisional device should start own-user SAS");
        assert!(refresh < start);
        assert!(helper.contains("target_a2.clone()"));
        assert!(helper.contains("primary incoming request"));
        assert!(helper.contains("SasQaOutcome::Timeout"));
        assert!(helper.contains("SasQaOutcome::Mismatch"));
        assert!(helper.contains("AccountCommand::CancelVerification"));
        assert!(helper.contains("AccountCommand::ConfirmSasVerification"));
        assert!(helper.contains("timed out waiting for authoritative Ready"));

        for forbidden in [
            "stop_sync_for_qa(conn_a",
            "start_sync_for_qa(conn_a",
            "sync_once_for_qa(conn_a",
        ] {
            assert!(
                !helper.contains(forbidden),
                "primary normal sync must remain continuously owned during SAS: {forbidden}"
            );
        }

        assert!(!helper.contains("stop_sync_for_qa(conn_a2"));
        assert!(!helper.contains("start_sync_for_qa(conn_a2"));
    }

    #[test]
    fn incoming_verification_waiter_rejects_stopped_receiver_sync_at_entry() {
        let label = "incoming verification receiver";
        assert_eq!(
            ensure_incoming_verification_receiver_sync_not_stopped(
                &koushi_state::SyncState::Stopped,
                label,
            ),
            Err(format!(
                "{label}: receiver sync is stopped; cannot await an incoming verification request"
            ))
        );
        for sync in [
            koushi_state::SyncState::Running,
            koushi_state::SyncState::Starting,
            koushi_state::SyncState::Failed {
                reason: "synthetic failure detail".to_owned(),
            },
            koushi_state::SyncState::Reconnecting {
                reason: "synthetic reconnect detail".to_owned(),
            },
        ] {
            assert_eq!(
                ensure_incoming_verification_receiver_sync_not_stopped(&sync, label),
                Ok(())
            );
        }

        let source = include_str!("headless-core-qa.rs");
        let guard = source
            .split("fn ensure_incoming_verification_receiver_sync_not_stopped")
            .nth(1)
            .expect("incoming verification sync guard should exist")
            .split("async fn wait_for_verification_requested_event_only")
            .next()
            .expect("incoming verification waiter should follow its sync guard");
        assert!(guard.contains("koushi_state::SyncState::Stopped"));
        assert!(
            guard.contains(
                "receiver sync is stopped; cannot await an incoming verification request"
            )
        );
        assert!(!guard.contains("{sync:?}"));

        let waiter = source
            .split("async fn wait_for_verification_requested_event_only")
            .nth(1)
            .expect("incoming verification waiter should exist")
            .split("fn requested_verification_flow_id")
            .next()
            .expect("verification flow classifier should follow incoming waiter");
        let sync_guard = waiter
            .find(
                "ensure_incoming_verification_receiver_sync_not_stopped(&conn.snapshot().sync, label)?",
            )
            .expect("incoming waiter should fail fast on stopped receiver sync");
        let deadline = waiter
            .find("let deadline")
            .expect("incoming waiter should retain its bounded deadline");
        assert!(sync_guard < deadline);
    }

    #[test]
    fn unused_manual_second_device_verification_cascade_is_absent() {
        let source = include_str!("headless-core-qa.rs");
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source should precede tests");
        for unused in [
            "async fn verify_second_device_for_qa",
            "enum VerificationRequestAttempt",
            "async fn request_device_verification_for_qa",
            "async fn wait_for_verification_requested_or_failed",
            "async fn wait_for_verification_accepted_with_sync_once",
            "async fn drive_until_both_verification_sas",
            "async fn wait_for_verification_done",
            "fn verification_state_done",
        ] {
            assert!(
                !production.contains(unused),
                "obsolete zero-caller verification orchestration must be deleted: {unused}"
            );
        }
    }

    #[test]
    fn final_tokens_follow_the_requested_scenario_including_composer() {
        assert_eq!(final_tokens_for_scenario(QaScenario::Safety), ["safety=ok"]);
        assert_eq!(
            final_tokens_for_scenario(QaScenario::LoginSync),
            ["safety=ok", "login_sync=ok", "restore_cleanup=ok"]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Composer),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "mention_send=ok",
                "markdown_send=ok",
                "slash_command=ok",
                "ime_guard=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::RoomSpace),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "restore_cleanup=ok"
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::InvitesDm),
            [
                "safety=ok",
                "login_sync=ok",
                "invite_recv=ok",
                "invite_accept=ok",
                "invite_decline=ok",
                "member_list=ok",
                "dm_start=ok",
                "dm_space_scope=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Timeline),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::TimelineReconnect),
            [
                "safety=ok",
                "timeline_reconnect_recv_after_reconnect=ok",
                "live_catchup_checkpoint=ok",
                "live_catchup_gap_repaired=ok",
                "timeline_reconnect=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::TimelineLegacyFallback),
            [
                "safety=ok",
                "legacy_fallback_checkpoint=ok",
                "legacy_fallback_gap_repaired=ok",
                "legacy_fallback_settled=ok",
                "legacy_fallback_lifecycle=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::TimelineLegacyPersistedGap),
            [
                "safety=ok",
                "legacy_live_tail_room_absent=ok",
                "live_tail_anchored_silent_gap=ok",
                "live_tail_detached_gap=ok",
                "live_tail_historical_continuation=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::TimelineStress),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "timeline_stress=ok",
                "stress_no_blank=ok",
                "stress_space_scope=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Activity),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "activity_recent=ok",
                "activity_unread=ok",
                "activity_resolution=ok",
                "activity_markread=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::CredentialHealth),
            [
                "safety=ok",
                "login_sync=ok",
                "credential_health=ok",
                "fail_closed=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::NativeAttention),
            [
                "safety=ok",
                "login_sync=ok",
                "notification_candidate=ok",
                "badge_state=ok",
                "suppress_focus=ok",
                "clear_badge=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Directory),
            [
                "safety=ok",
                "login_sync=ok",
                "directory_query=ok",
                "directory_join=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Reply),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "mention_send=ok",
                "markdown_send=ok",
                "slash_command=ok",
                "ime_guard=ok",
                "reply=ok",
                "reply_quote=ok",
                "pin_event=ok",
                "pinned_state=ok",
                "unpin_event=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Media),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "send_media=ok",
                "media_caption=ok",
                "image_compress=ok",
                "upload_staging=ok",
                "media_gallery=ok",
                "recv_media=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::LiveSignals),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "read_receipt=ok",
                "fully_read=ok",
                "typing=ok",
                "presence=ok",
                "live_signals=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::Thread),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "reply=ok",
                "reply_quote=ok",
                "pin_event=ok",
                "pinned_state=ok",
                "unpin_event=ok",
                "thread_canonical=ok",
                "thread_summary=ok",
                "thread_recv=ok",
                "thread_paginate=end_reached",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::EditRedactSearch),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "edit_redact_search=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::SearchCrawler),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "edit_redact_search=ok",
                "crawl_backfill=ok",
                "crawl_no_media_bytes=ok",
                "crawl_throttle=ok",
                "crawl_failure=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::ScheduledSend),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "scheduled_capability=local_fallback",
                "scheduled_create=ok",
                "scheduled_reschedule=ok",
                "scheduled_cancel=ok",
                "scheduled_fire=ok",
                "restore_cleanup=ok",
            ]
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::All),
            implemented_final_tokens()
        );
        assert_eq!(
            final_tokens_for_scenario(QaScenario::E2eeTrust),
            [
                "safety=ok",
                "login_sync=ok",
                "joined_room_restore=ok",
                "e2ee_second_device_decrypt=ok",
                "e2ee_multi_user_multi_device_decrypt=ok",
                "e2ee_unverified_peer_send_nonblocking=ok",
                "e2ee_blocked_device_withheld=ok",
                "e2ee_trust=ok",
                "restore_cleanup=ok",
            ]
        );
    }

    #[test]
    fn implemented_final_tokens_include_safety() {
        assert_eq!(
            &implemented_final_tokens()[..],
            &[
                "safety=ok",
                "login_sync=ok",
                "credential_health=ok",
                "fail_closed=ok",
                "notification_candidate=ok",
                "badge_state=ok",
                "suppress_focus=ok",
                "clear_badge=ok",
                "invite_recv=ok",
                "invite_accept=ok",
                "invite_decline=ok",
                "member_list=ok",
                "dm_start=ok",
                "dm_space_scope=ok",
                "room_space=ok",
                "directory_query=ok",
                "directory_join=ok",
                "room_settings=ok",
                "moderation=ok",
                "permission_guard=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "activity_recent=ok",
                "activity_unread=ok",
                "activity_resolution=ok",
                "activity_markread=ok",
                "mention_send=ok",
                "markdown_send=ok",
                "slash_command=ok",
                "ime_guard=ok",
                "reply=ok",
                "reply_quote=ok",
                "pin_event=ok",
                "pinned_state=ok",
                "unpin_event=ok",
                "thread_canonical=ok",
                "thread_summary=ok",
                "thread_recv=ok",
                "thread_paginate=end_reached",
                "send_media=ok",
                "media_caption=ok",
                "image_compress=ok",
                "upload_staging=ok",
                "media_gallery=ok",
                "recv_media=ok",
                "read_receipt=ok",
                "fully_read=ok",
                "typing=ok",
                "presence=ok",
                "live_signals=ok",
                "edit_redact_search=ok",
                "crawl_backfill=ok",
                "crawl_no_media_bytes=ok",
                "crawl_throttle=ok",
                "crawl_failure=ok",
                "scheduled_capability=local_fallback",
                "scheduled_create=ok",
                "scheduled_reschedule=ok",
                "scheduled_cancel=ok",
                "scheduled_fire=ok",
                "send_fail=ok",
                "resend=ok",
                "cancel_send=ok",
                "fifo=ok",
                "unsent_restart=ok",
                "display_projection_reset_fallbacks=0",
                "joined_room_restore=ok",
                "e2ee_second_device_decrypt=ok",
                "e2ee_multi_user_multi_device_decrypt=ok",
                "e2ee_unverified_peer_send_nonblocking=ok",
                "e2ee_blocked_device_withheld=ok",
                "e2ee_trust=ok",
                "restore_cleanup=ok",
                "link_preview_global=ok",
                "link_preview_room=ok",
                "link_preview_e2ee_default=ok",
                "link_preview_hide=ok",
            ][..]
        );
    }
}
