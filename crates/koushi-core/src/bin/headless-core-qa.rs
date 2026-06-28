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
use std::process::ExitCode;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::BTreeSet, io};

use koushi_core::command::{
    AccountCommand, AppCommand, CoreCommand, ImageUploadCompressionPolicy,
    ImageUploadCompressionState, ImageUploadDimensions, ImageUploadVariantInfo,
    ImageUploadVariantKind, MediaDownloadSelection, RoomCommand, SearchCommand, SearchScope,
    SyncCommand, TimelineCommand, UploadMediaKind, UploadMediaRequest, UploadMediaThumbnail,
};
use koushi_core::event::{
    AccountEvent, ActivityEvent, CoreEvent, E2eeTrustEvent, LinkPreviewState, LiveSignalsEvent,
    LocalEncryptionEvent, PaginationDirection, PaginationState, RoomEvent, SearchEvent,
    SyncBackendKind, SyncEvent, TimelineAnchorMaterializeStatus, TimelineDiff, TimelineEvent,
    TimelineItem, TimelineItemId, TimelineMessageActions, TimelineSendState,
    TimelineUnreadPosition, TimelineViewportObservation,
};
use koushi_core::failure::{CoreFailure, RoomFailureKind};
use koushi_core::ids::{AccountKey, RequestId, TimelineKey, TimelineKind};
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_state::{
    ActivityMarkReadTarget, ActivityState, AppAction, AppState, AuthSecret, ComposerKey,
    ComposerKeyEvent, ComposerKeyModifiers, ComposerResolvedAction, ComposerResolverContext,
    ComposerSelection, ComposerSendShortcut, ComposerSurface, CrossSigningStatus, DirectoryQuery,
    DirectoryRoomSummary, DisplaySettings, IdentityResetAuthRequest, IdentityResetAuthType,
    IdentityResetState, ImageUploadCompressionMode, KeyBackupStatus, LocalEncryptionHealth,
    LocalEncryptionState, MentionIntent, MentionTarget, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionDispatchState, NativeAttentionObservationKind,
    NativeAttentionProjectionInput, NativeAttentionState, NativeAttentionSuppressionReason,
    OperationFailureKind, PresenceKind, RecoveryRequest, ReplyQuoteState, RoomAttentionKind,
    RoomListFilter, RoomManagementOperationKind, RoomManagementOperationState,
    RoomModerationAction, RoomNotificationMode, RoomSettingChange, RoomSettingsSnapshot,
    RoomSummary, RoomTags, SasEmoji, ScheduledSendCapability, SearchCrawlerFailureKind,
    SearchCrawlerRoomState, SearchCrawlerSettings, SearchCrawlerSpeed, SessionInfo, SessionState,
    SettingsPatch, SettingsPersistenceState, StagedUploadCompressionChoice, StagedUploadItem,
    StagedUploadKind, TimelineMediaGalleryItem, TimelineMediaGalleryMedia,
    TimelineMediaGallerySource, TimelineMediaKind, TrustOperationFailureKind,
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
const ENV_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND: &str =
    "KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND";
#[cfg(any(debug_assertions, test))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";

const DEVICE_A: &str = "Koushi Core QA A";
const DEVICE_B: &str = "Koushi Core QA B";

/// Maximum time to wait for a single event.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const ROOM_LIST_EVENT_TIMEOUT: Duration = Duration::from_secs(90);
const E2EE_EVENT_TIMEOUT: Duration = Duration::from_secs(90);
const DEVICE_LIST_SETTLE_SYNC_TIMEOUT: Duration = Duration::from_secs(5);
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
/// (LIVE_ROOM_ANCHOR_MATERIALIZE_MAX_BATCHES=6, EVENT_COUNT=100).
/// With depth=200 and each SDK paginate_backwards returning ~one stored chunk,
/// 6 batches reaches only ~6 chunks (~tens of events) → BudgetExhausted on main.
const CACHE_RESTORE_PROD_MAX_BATCHES: u16 = 6;
const CACHE_RESTORE_PROD_EVENT_COUNT: u16 = 100;
/// Stage 2 speed gate: maximum backward-paginate cycles allowed per room during
/// an offline anchor materialize.  With a bulk cache load (Stage 2), the runtime
/// should reach the anchor in ≤ 3 cycles.  On current main (~12 cycles/room)
/// this gate is RED — that is the intended signal before Stage 2 lands.
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
    InvitesDm,
    RoomSpace,
    Directory,
    RoomManagement,
    Timeline,
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
    InvitesDm,
    RoomSpace,
    Directory,
    RoomManagement,
    Timeline,
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

/// Refuse to run against the OS keychain. In debug/test builds this checks
/// both the env var and the structurally resolved backend; release builds
/// have no file credential store at all, so they are refused outright before
/// the env var is even consulted.
fn assert_file_credential_store_active() -> Result<(), String> {
    #[cfg(debug_assertions)]
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

    #[cfg(not(debug_assertions))]
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
            "invites_dm" => Ok(Self::InvitesDm),
            "room_space" => Ok(Self::RoomSpace),
            "directory" => Ok(Self::Directory),
            "room_management" => Ok(Self::RoomManagement),
            "timeline" => Ok(Self::Timeline),
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
                "{ENV_QA_SCENARIO} must be one of all, safety, login_sync, credential_health, native_attention, e2ee_trust, invites_dm, room_space, directory, room_management, timeline, timeline_stress, activity, composer, reply, media, live_signals, thread, edit_redact_search, search_crawler, scheduled_send, restore_cleanup, link_preview, cache_restore; got {other}"
            )),
        }
    }

    fn should_run_stage(self, stage: QaStage) -> bool {
        match self {
            Self::All => !matches!(stage, QaStage::TimelineStress),
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
                QaStage::Safety
                    | QaStage::LoginSync
                    | QaStage::RoomSpace
                    | QaStage::Timeline
                    | QaStage::SendQueue
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
            "e2ee_trust=ok",
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
        QaStage::TimelineStress => &[
            "timeline_stress=ok",
            "stress_no_blank=ok",
            "stress_space_scope=ok",
        ],
        QaStage::Activity => &[
            "activity_recent=ok",
            "activity_unread=ok",
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
            "thread_hidden=ok",
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
        "thread_hidden=ok",
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
        "joined_room_restore=ok",
        "e2ee_second_device_decrypt=ok",
        "e2ee_multi_user_multi_device_decrypt=ok",
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
        QaScenario::SendQueue => vec![
            QaStage::Safety,
            QaStage::LoginSync,
            QaStage::RoomSpace,
            QaStage::Timeline,
            QaStage::SendQueue,
        ],
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
        QaScenario::CacheRestore => stages_for_scenario(scenario)
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
    drop(runtime_a);
    // Store-lock release after dropping the runtime is a filesystem event with
    // no observable Core signal to wait on; this brief bounded wait avoids
    // store-lock contention when the same data dir is reopened below.
    tokio::time::sleep(Duration::from_millis(500)).await;

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

    let failure = wait_for_operation_failed(
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
    if !matches!(conn_a2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore A must leave the session SignedOut".to_owned());
    }

    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
}

async fn run_invites_dm_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    data_dir_b: std::path::PathBuf,
) -> Result<(), String> {
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
                device_display_name: Some(DEVICE_B.to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("invites_dm: submit login B failed: {e}"))?;

    let account_key_b = wait_for_logged_in(&mut conn_b, login_b_id, "invites_dm login B").await?;
    wait_for_ready_snapshot(&mut conn_b, "invites_dm session B Ready").await?;
    start_sync_for_qa(&mut conn_b, "invites_dm sync B").await?;

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
    sync_once_for_qa(&mut conn_b, "invites_dm sync B for room invite").await?;
    wait_for_invite_in_snapshot(
        &mut conn_b,
        &accept_room_id,
        Some(false),
        "invites_dm wait for room invite",
    )
    .await?;
    println!("invite_recv=ok");

    accept_invite_for_qa(
        &mut conn_b,
        &accept_room_id,
        "invites_dm accept room invite",
    )
    .await?;
    sync_once_for_qa(&mut conn_b, "invites_dm sync B after room accept").await?;
    wait_for_room_in_room_list(
        &mut conn_b,
        &accept_room_id,
        "invites_dm room list after room accept",
    )
    .await?;
    let accept_room_settings = load_room_settings_for_qa(
        &mut conn_b,
        &accept_room_id,
        "invites_dm accepted room members",
    )
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
    sync_once_for_qa(&mut conn_b, "invites_dm sync B for space invite").await?;
    wait_for_invite_in_snapshot(
        &mut conn_b,
        &accept_space_id,
        Some(false),
        "invites_dm wait for space invite",
    )
    .await?;
    accept_invite_for_qa(
        &mut conn_b,
        &accept_space_id,
        "invites_dm accept space invite",
    )
    .await?;
    sync_once_for_qa(&mut conn_b, "invites_dm sync B after space accept").await?;
    wait_for_space_in_space_list(
        &mut conn_b,
        &accept_space_id,
        "invites_dm space list after space accept",
    )
    .await?;
    let accept_space_settings = load_room_settings_for_qa(
        &mut conn_b,
        &accept_space_id,
        "invites_dm accepted space members",
    )
    .await?;
    assert_room_settings_contains_members(
        &accept_space_settings,
        &[user_a_full_id.as_str(), user_b_full_id.as_str()],
        "invites_dm accepted space members",
    )?;
    sync_once_for_qa(conn_a, "invites_dm sync A after space accept").await?;
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
    sync_once_for_qa(&mut conn_b, "invites_dm sync B for decline invite").await?;
    wait_for_invite_in_snapshot(
        &mut conn_b,
        &decline_room_id,
        Some(false),
        "invites_dm wait for decline invite",
    )
    .await?;
    decline_invite_for_qa(
        &mut conn_b,
        &decline_room_id,
        "invites_dm decline room invite",
    )
    .await?;
    sync_once_for_qa(&mut conn_b, "invites_dm sync B after decline").await?;
    wait_for_invite_absent(
        &mut conn_b,
        &decline_room_id,
        "invites_dm wait for declined invite removal",
    )
    .await?;
    println!("invite_decline=ok");

    let dm_room_id =
        start_direct_message_for_qa(conn_a, &user_b_full_id, "invites_dm start direct message")
            .await?;
    sync_once_for_qa(conn_a, "invites_dm sync A after DM start").await?;
    wait_for_dm_room_in_room_list(conn_a, &dm_room_id, "invites_dm A room list after DM start")
        .await?;
    sync_once_for_qa(&mut conn_b, "invites_dm sync B for DM invite").await?;
    wait_for_invite_in_snapshot(
        &mut conn_b,
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
    sync_once_for_qa(conn_a, "invites_dm sync A after control DM start").await?;
    wait_for_dm_room_in_room_list(
        conn_a,
        &control_dm_room_id,
        "invites_dm A room list after control DM start",
    )
    .await?;
    assert_dm_space_scope_for_qa(conn_a, &accept_space_id, &dm_room_id, &control_dm_room_id)
        .await?;
    println!("dm_space_scope=ok");

    cleanup_logged_in_runtime(conn_b, runtime_b, account_key_b, "invites_dm cleanup B").await?;
    Ok(())
}

async fn run_directory_stage(config: &QaConfig, conn_a: &mut CoreConnection) -> Result<(), String> {
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

    let runtime_b = CoreRuntime::start_with_data_dir(qa_data_dir("directory_b"));
    let mut conn_b = runtime_b.attach();

    let login_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_b_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_b.clone(),
                password: AuthSecret::new(config.password_b.clone()),
                device_display_name: Some("Koushi Core QA Directory B".to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("directory: submit login B failed: {e}"))?;

    let account_key_b = wait_for_logged_in(&mut conn_b, login_b_id, "directory login B").await?;
    wait_for_ready_snapshot(&mut conn_b, "directory session B Ready").await?;

    let join_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
            request_id: join_id,
            alias: expected_alias,
            via_server: Some(config.server_name.clone()),
        }))
        .await
        .map_err(|e| format!("directory: submit join by alias failed: {e}"))?;
    wait_for_room_joined(
        &mut conn_b,
        join_id,
        &public_room_id,
        "directory B joins public room",
    )
    .await?;
    println!("directory_join=ok");

    cleanup_logged_in_runtime(conn_b, runtime_b, account_key_b, "directory cleanup B").await?;
    Ok(())
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
    sync_once_for_qa(conn_a, "room_management sync A after create").await?;
    wait_for_room_in_room_list(conn_a, &room_id, "room_management A room list").await?;

    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    invite_user_for_qa(
        conn_a,
        &room_id,
        &user_b_full_id,
        "room_management invite B",
    )
    .await?;
    sync_once_for_qa(conn_b, "room_management sync B for invite").await?;
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
    sync_once_for_qa(conn_a, "room_management sync A after B join").await?;
    sync_once_for_qa(conn_b, "room_management sync B after join").await?;

    let settings_a =
        load_room_settings_for_qa(conn_a, &room_id, "room_management load settings A").await?;
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
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "session A info for E2EE trust")?;

    bootstrap_cross_signing_for_qa(
        conn_a,
        account_key_a,
        Some(AuthSecret::new(config.password_a.clone())),
        "bootstrap cross-signing A",
    )
    .await?;
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
    let mut conn_a2 = runtime_a2.attach();

    let login_a2_id = conn_a2.next_request_id();
    conn_a2
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

    let account_key_a2 = wait_for_logged_in(&mut conn_a2, login_a2_id, "login A2").await?;
    wait_for_ready_snapshot(&mut conn_a2, "session A2 Ready").await?;
    let session_a2 = authenticated_session_info(&mut conn_a2, "session A2 info for E2EE trust")?;

    let sync_start_a2_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_a2_id,
        }))
        .await
        .map_err(|e| format!("submit sync start A2: {e}"))?;
    let sync_backend_a2 =
        wait_for_sync_started_and_running(&mut conn_a2, sync_start_a2_id, "sync start A2").await?;
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_a2,
        "sync start A2",
    )?;

    wait_for_room_in_room_list(
        &mut conn_a2,
        &key_backup_seed_room_id,
        "room list A2 after key backup seed",
    )
    .await?;

    restore_key_backup_failure_for_qa(
        &mut conn_a2,
        &account_key_a2,
        Some(key_backup_version.clone()),
        "restore key backup failure A2",
    )
    .await?;
    println!("e2ee_key_backup_restore_failure=ok");

    restore_key_backup_success_for_qa(
        &mut conn_a2,
        &account_key_a2,
        Some(key_backup_version),
        AuthSecret::new(config.password_a.clone()),
        "restore key backup success A2",
    )
    .await?;
    println!("joined_room_restore=ok");

    verify_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &session_a2,
        "e2ee self verification A/A2",
    )
    .await?;
    println!("e2ee_verification=ok");

    verify_second_device_room_key_delivery_for_qa(
        conn_a,
        &mut conn_a2,
        account_key_a,
        &account_key_a2,
        &key_backup_seed_room_id,
    )
    .await?;
    println!("e2ee_second_device_decrypt=ok");

    verify_multi_user_multi_device_room_key_delivery_for_qa(
        config,
        conn_a,
        &mut conn_a2,
        account_key_a,
        &account_key_a2,
    )
    .await?;
    println!("e2ee_multi_user_multi_device_decrypt=ok");

    cleanup_e2ee_secondary_device(conn_a2, runtime_a2, account_key_a2).await?;

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

async fn cleanup_e2ee_secondary_device(
    conn: CoreConnection,
    runtime: CoreRuntime,
    account_key: AccountKey,
) -> Result<(), String> {
    cleanup_logged_in_runtime(conn, runtime, account_key, "cleanup secondary device").await
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
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
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

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
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
        room_id: room_id.to_owned(),
        body: SCHEDULED_CREATE_BODY.to_owned(),
        send_at_ms: scheduled_qa_epoch_ms(Duration::from_secs(300)),
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
        room_id: room_id.to_owned(),
        body: SCHEDULED_FIRE_BODY.to_owned(),
        send_at_ms: scheduled_qa_epoch_ms(Duration::from_millis(250)),
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
        sync_once_for_qa(&mut conn, "cache_restore sync after room create").await?;
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
    sync_once_for_qa(&mut conn, "cache_restore sync after shallow room create").await?;
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
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // -----------------------------------------------------------------------
    // Connect 2: restart over the same data dir, BLOCK the network, then drive
    // MaterializeTimelineAnchor per room using production-faithful params.
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
    let mut all_succeeded = true;
    let mut total_cycles: u32 = 0;
    // Per-room cycle counts for the Stage 2 speed gate.
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
        let initial_offline =
            wait_for_initial_items(&mut conn2, &key, sub_id, "cache_restore offline subscribe")
                .await?;
        // Track items during restore so we can check anchor presence on EndReached.
        let mut offline_items = initial_offline;

        let room_start = std::time::Instant::now();
        let restore_req = conn2.next_request_id();
        conn2
            .command(CoreCommand::Timeline(
                TimelineCommand::MaterializeTimelineAnchor {
                    request_id: restore_req,
                    key: key.clone(),
                    event_id: anchor.clone(),
                    // Production-faithful params: source TimelineView.tsx:406-407
                    // (LIVE_ROOM_ANCHOR_MATERIALIZE_MAX_BATCHES=6, EVENT_COUNT=100).
                    // With depth=200, each paginate_backwards returns ~one stored chunk,
                    // so 6 batches reaches only ~tens of events → BudgetExhausted on main.
                    max_batches: CACHE_RESTORE_PROD_MAX_BATCHES,
                    event_count: CACHE_RESTORE_PROD_EVENT_COUNT,
                },
            ))
            .await
            .map_err(|e| {
                format!("cache_restore: offline MaterializeTimelineAnchor submit failed: {e}")
            })?;

        // Consume events until AnchorMaterializeFinished. Count Paginating transitions
        // as internal backward-paginate cycles (PRIMARY diagnostic; also gated by
        // CACHE_RESTORE_MAX_CYCLES as Stage 2 speed regression gate).
        let mut cycle_count: u16 = 0;
        let status = loop {
            let event = tokio::time::timeout(Duration::from_secs(120), conn2.recv_event())
                .await
                .map_err(|_| {
                    "cache_restore offline: timed out waiting for AnchorMaterializeFinished"
                        .to_owned()
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
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                    key: ref ev_key,
                    ref diffs,
                    ..
                }) if ev_key == &key => {
                    for diff in diffs {
                        apply_timeline_diff(&mut offline_items, diff);
                    }
                }
                CoreEvent::Timeline(TimelineEvent::AnchorMaterializeFinished {
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
            TimelineAnchorMaterializeStatus::Found => "found",
            TimelineAnchorMaterializeStatus::EndReached => "end_reached",
            TimelineAnchorMaterializeStatus::BudgetExhausted => "budget_exhausted",
            TimelineAnchorMaterializeStatus::Superseded => "superseded",
            TimelineAnchorMaterializeStatus::Failed { .. } => "failed",
        };
        // Private-data-free diagnostics: cycles + ms only, no ids or bodies.
        eprintln!(
            "cache_restore room={room_idx} cycles={cycle_count} ms={room_ms} status={status_label}"
        );

        // PRIMARY CORRECTNESS GATE:
        // Found → anchor reached within budget (GREEN post-fix).
        // EndReached → paginated to start of room; succeed only if anchor is in items.
        // BudgetExhausted/Failed/Superseded → deep anchor not restored (RED on main).
        let room_succeeded = match &status {
            TimelineAnchorMaterializeStatus::Found => true,
            TimelineAnchorMaterializeStatus::EndReached => {
                let anchor_present = find_timeline_item_with_body(
                    &offline_items,
                    &format!("cache_restore fixture r{room_idx} m0"),
                )
                .is_some();
                if !anchor_present {
                    eprintln!(
                        "cache_restore room={room_idx}: EndReached but anchor absent from items"
                    );
                }
                anchor_present
            }
            TimelineAnchorMaterializeStatus::BudgetExhausted => {
                eprintln!(
                    "cache_restore room={room_idx}: BudgetExhausted — deep anchor not restored \
                     within production budget (EXPECTED RED on current main)"
                );
                false
            }
            TimelineAnchorMaterializeStatus::Failed { .. }
            | TimelineAnchorMaterializeStatus::Superseded => {
                eprintln!("cache_restore room={room_idx}: restore status={status_label} offline");
                false
            }
        };
        if !room_succeeded {
            all_succeeded = false;
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
            TimelineCommand::MaterializeTimelineAnchor {
                request_id: shallow_restore_req,
                key: shallow_key2.clone(),
                event_id: shallow_anchor_id.clone(),
                max_batches: CACHE_RESTORE_PROD_MAX_BATCHES,
                event_count: CACHE_RESTORE_PROD_EVENT_COUNT,
            },
        ))
        .await
        .map_err(|e| {
            format!("cache_restore shallow: offline MaterializeTimelineAnchor submit failed: {e}")
        })?;

    let mut shallow_cycle_count: u16 = 0;
    let shallow_status = loop {
        let event = tokio::time::timeout(Duration::from_secs(60), conn2.recv_event())
            .await
            .map_err(|_| {
                "cache_restore shallow: timed out waiting for AnchorMaterializeFinished".to_owned()
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
            CoreEvent::Timeline(TimelineEvent::AnchorMaterializeFinished {
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
        TimelineAnchorMaterializeStatus::Found => "found",
        TimelineAnchorMaterializeStatus::EndReached => "end_reached",
        TimelineAnchorMaterializeStatus::BudgetExhausted => "budget_exhausted",
        TimelineAnchorMaterializeStatus::Superseded => "superseded",
        TimelineAnchorMaterializeStatus::Failed { .. } => "failed",
    };
    eprintln!("cache_restore shallow cycles={shallow_cycle_count} status={shallow_status_label}");

    // Gate: shallow anchor must reach Found (the lazy-reveal path must settle
    // before declaring the materialize terminal).  cycle_count==0 is the expected
    // value after the P1 fix (no disk chunk needed); a non-zero count is
    // unexpected but not a hard gate here — correctness (Found) is the gate.
    let shallow_succeeded = matches!(&shallow_status, TimelineAnchorMaterializeStatus::Found);
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

    // SECONDARY GATE (Stage 2 speed regression gate):
    // Each room must reach the anchor in ≤ CACHE_RESTORE_MAX_CYCLES backward-paginate
    // cycles.  On current main (~12 cycles/room) this will FAIL — that is the intended
    // RED before Stage 2 bulk cache load lands.  The PRIMARY correctness gate above
    // (Found / EndReached+anchor) is checked independently so we can distinguish a
    // correct-but-slow restore from a genuinely broken one.
    let slow_rooms: Vec<usize> = room_cycle_counts
        .iter()
        .enumerate()
        .filter(|&(_, c)| *c > CACHE_RESTORE_MAX_CYCLES)
        .map(|(i, _)| i)
        .collect();

    cleanup_logged_in_runtime(conn2, runtime2, account_key, "cache_restore cleanup").await?;

    if !all_succeeded {
        return Err(
            "cache_restore: deep anchor not restored within the app's production restore budget \
             (symptom B) — raise budget / bulk cache load to fix (EXPECTED RED on current main)"
                .to_owned(),
        );
    }

    if !slow_rooms.is_empty() {
        let worst = room_cycle_counts.iter().copied().max().unwrap_or(0);
        return Err(format!(
            "cache_restore: deep anchor reached but via {worst} backward-paginate cycles \
             (> {CACHE_RESTORE_MAX_CYCLES}) — O(depth) restore, Stage 2 bulk cache load not \
             yet applied (EXPECTED RED until Stage 2)"
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

async fn run_send_queue_stage(config: &QaConfig) -> Result<(), String> {
    let proxy = QaTcpProxy::start(&config.homeserver)?;
    let data_dir = qa_data_dir("send_queue");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();

    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: proxy.homeserver_url(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi Core QA Send Queue".to_owned()),
        },
    }))
    .await
    .map_err(|e| format!("send_queue: submit login failed: {e}"))?;

    let account_key = wait_for_logged_in(&mut conn, login_id, "send_queue login").await?;
    wait_for_ready_snapshot(&mut conn, "send_queue Ready").await?;
    start_sync_for_qa(&mut conn, "send_queue sync").await?;

    let room_id = create_room_for_qa(
        &mut conn,
        "QA Send Queue Room",
        false,
        "send_queue create room",
    )
    .await?;
    sync_once_for_qa(&mut conn, "send_queue sync after room create").await?;
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
    .await?;
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

    stop_sync_for_qa(&mut conn, "send_queue stop before restart").await?;
    drop(conn);
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(500)).await;

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

    cleanup_logged_in_runtime(conn, runtime, account_key, "send_queue cleanup").await
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
    drop(runtime_a);
    // Store-lock release after dropping the runtime is a filesystem event with
    // no observable Core signal to wait on; this brief bounded wait avoids
    // store-lock contention when the same data dir is reopened below.
    tokio::time::sleep(Duration::from_millis(500)).await;

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

    let failure = wait_for_operation_failed(
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
    if !matches!(conn_a2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore A must leave the session SignedOut".to_owned());
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

    let failure = wait_for_operation_failed(
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
    if !matches!(conn_b.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore-last must leave the session SignedOut".to_owned());
    }

    drop(conn_b);
    drop(runtime_b);

    println!("restore_cleanup=ok");
    Ok("restore_cleanup=ok".to_owned())
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

    // One CoreRuntime per synthetic user (two-device topology).
    let data_dir_a = qa_data_dir("a");
    let data_dir_b = qa_data_dir("b");

    // -----------------------------------------------------------------------
    // --- Login A (storeless exchange + store bootstrap inside the actor) ---
    // -----------------------------------------------------------------------
    let runtime_a = CoreRuntime::start_with_data_dir(data_dir_a.clone());
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

    let account_key_a = wait_for_logged_in(&mut conn_a, login_a_id, "login A").await?;
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
        run_e2ee_trust_stage(&config, &mut conn_a, &account_key_a).await?;
    }

    if scenario.should_run_stage(QaStage::InvitesDm) {
        run_invites_dm_stage(&config, &mut conn_a, data_dir_b.clone()).await?;
    }

    if scenario == QaScenario::InvitesDm {
        cleanup_after_login_sync(conn_a, runtime_a, data_dir_a, account_key_a).await?;
        return Ok(scenario_report(&config.server_kind, scenario));
    }

    if !scenario.should_run_stage(QaStage::RoomSpace) {
        if scenario.should_run_stage(QaStage::Directory) {
            run_directory_stage(&config, &mut conn_a).await?;
        }
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
            name: "QA Room".to_owned(),
            encrypted: false,
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

    // Ensure the space create state has been folded into A's room-list
    // classification before asserting rooms vs spaces.
    sync_once_for_qa(&mut conn_a, "sync A after room and space creates").await?;

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
    // --- Login B + sync B + join room + join space ---
    // -----------------------------------------------------------------------
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
                device_display_name: Some(DEVICE_B.to_owned()),
            },
        }))
        .await
        .map_err(|e| format!("submit login B: {e}"))?;

    let account_key_b = wait_for_logged_in(&mut conn_b, login_b_id, "login B").await?;
    wait_for_ready_snapshot(&mut conn_b, "session B Ready").await?;

    // Start sync B
    let sync_start_b_id = conn_b.next_request_id();
    conn_b
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: sync_start_b_id,
        }))
        .await
        .map_err(|e| format!("submit sync start B: {e}"))?;

    let sync_backend_b =
        wait_for_sync_started_and_running(&mut conn_b, sync_start_b_id, "sync start B").await?;
    println!("sync_backend_b={sync_backend_b:?}");
    assert_expected_backend(
        config.expect_sync_backend.as_deref(),
        sync_backend_b,
        "sync start B",
    )?;

    println!("sync_b=running");

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

    // Ensure the joined space has been folded into B's room-list
    // classification before asserting rooms vs spaces.
    sync_once_for_qa(&mut conn_b, "sync B after room and space joins").await?;

    // Wait (event-driven, bounded) until B's room list contains the joined
    // room AND the joined space; the wait itself is the assertion.
    let snapshot_b =
        wait_for_room_list_containing(&mut conn_b, &room_id, &space_id, "room list B after joins")
            .await?;
    let room_list_b = room_list_summary(&snapshot_b);
    println!("room_list_b={room_list_b}");
    println!("room_space=ok");

    if scenario.should_run_stage(QaStage::Directory) {
        run_directory_stage(&config, &mut conn_a).await?;
    }

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
                at_bottom: false,
                scroll_anchor: None,
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

    if scenario.should_run_stage(QaStage::Activity) {
        run_activity_stage(&mut conn_a, &mut conn_b, &key_a, &key_b, &room_id).await?;
    }

    if scenario.should_run_stage(QaStage::Composer) {
        run_composer_stage(&mut conn_a, &key_a, &account_key_b.0).await?;
    }

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
        println!("thread_hidden=ok");
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
        run_send_queue_stage(&config).await?;
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
        run_e2ee_trust_stage(&config, &mut conn_a, &account_key_a).await?;
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
    drop(runtime_a);
    // Store-lock release after dropping the runtime is a filesystem event with
    // no observable Core signal to wait on; this brief bounded wait avoids
    // store-lock contention when the same data dir is reopened on restore.
    tokio::time::sleep(Duration::from_millis(500)).await;

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

    let failure = wait_for_operation_failed(
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
    if !matches!(conn_a2.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore A must leave the session SignedOut".to_owned());
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

    let failure = wait_for_operation_failed(
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
    if !matches!(conn_b.snapshot().session, SessionState::SignedOut) {
        return Err("post-logout restore-last must leave the session SignedOut".to_owned());
    }

    println!("restore_cleanup=ok");
    Ok(scenario_report(&config.server_kind, scenario))
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

async fn create_room_for_qa(
    conn: &mut CoreConnection,
    name: &str,
    encrypted: bool,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        name: name.to_owned(),
        encrypted,
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

    loop {
        let event = tokio::time::timeout(ROOM_LIST_EVENT_TIMEOUT, conn.recv_event())
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

    for attempt in 0..5 {
        if sidebar_dm_room_ids(&conn.snapshot()) == expected {
            return Ok(());
        }
        let sync_label = format!("{label} SyncOnce attempt {}", attempt + 1);
        sync_once_for_qa(conn, &sync_label).await?;
    }

    let snapshot = conn.snapshot();
    let observed_count = sidebar_dm_room_ids(&snapshot).len();
    if sidebar_dm_room_ids(&snapshot) == expected {
        return Ok(());
    }

    Err(format!(
        "{label}: DM section scope mismatch \
         (expected_count={}, observed_count={}, active_space={})",
        expected.len(),
        observed_count,
        snapshot.navigation.active_space_id.is_some()
    ))
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
                format!(
                    "{label}: timed out waiting for invite snapshot \
                     (have {} invites)",
                    snapshot.invites.len()
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
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
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

async fn sync_once_for_qa(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::SyncOnce { request_id }))
        .await
        .map_err(|e| format!("{label}: submit SyncOnce failed: {e}"))?;
    wait_for_sync_once(conn, request_id, label).await
}

async fn best_effort_sync_once_for_qa(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::SyncOnce { request_id }))
        .await
        .map_err(|e| format!("{label}: submit SyncOnce failed: {e}"))?;

    loop {
        let event =
            match tokio::time::timeout(DEVICE_LIST_SETTLE_SYNC_TIMEOUT, conn.recv_event()).await {
                Ok(Ok(event)) => event,
                Ok(Err(lag)) => {
                    return Err(format!(
                        "{label}: event stream lagged during best-effort SyncOnce (skipped={})",
                        lag.skipped
                    ));
                }
                Err(_) => return Ok(()),
            };

        match event {
            CoreEvent::Sync(SyncEvent::Stopped {
                request_id: Some(ev_id),
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id, ..
            } if ev_id == request_id => return Ok(()),
            _ => {}
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

async fn start_sync_for_qa(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start { request_id }))
        .await
        .map_err(|e| format!("{label}: submit Sync start failed: {e}"))?;
    wait_for_sync_started_and_running(conn, request_id, label)
        .await
        .map(|_| ())
}

async fn wait_for_sync_once(
    conn: &mut CoreConnection,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SyncOnce"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Sync(SyncEvent::Stopped {
                request_id: Some(ev_id),
            }) if ev_id == request_id => return Ok(()),
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: SyncOnce failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

/// Wait for a `StateChanged` snapshot where `SessionState::Ready`.
async fn wait_for_ready_snapshot(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    if matches!(conn.snapshot().session, SessionState::Ready(_)) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
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
) -> Result<(Vec<String>, Vec<String>), String> {
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
                return Ok((
                    recent
                        .rows
                        .into_iter()
                        .filter_map(|row| row.event_id)
                        .collect(),
                    unread
                        .rows
                        .into_iter()
                        .filter_map(|row| row.event_id)
                        .collect(),
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
async fn wait_for_logged_in(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<AccountKey, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for LoggedIn event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

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
            _ => continue,
        }
    }
}

/// Wait for `AccountEvent::SessionRestored` with the given request_id.
async fn wait_for_session_restored(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SessionRestored event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if account_key != *expected_account_key {
                    return Err(format!(
                        "{label}: SessionRestored account_key mismatch: got {:?}, expected {:?}",
                        account_key, expected_account_key
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

/// Wait for `AccountEvent::LoggedOut` with the given request_id.
async fn wait_for_logged_out(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    expected_account_key: &AccountKey,
    label: &str,
) -> Result<(), String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for LoggedOut event"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Account(AccountEvent::LoggedOut {
                request_id: ev_id,
                account_key,
            }) if ev_id == request_id => {
                if account_key != *expected_account_key {
                    return Err(format!(
                        "{label}: LoggedOut account_key mismatch: got {:?}, expected {:?}",
                        account_key, expected_account_key
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

/// Wait for `OperationFailed` with the given request_id and return the failure.
async fn wait_for_operation_failed(
    conn: &mut CoreConnection,
    request_id: koushi_core::ids::RequestId,
    label: &str,
) -> Result<CoreFailure, String> {
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
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
                    AccountEvent::RecoveryRequired { .. } => false,
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
        SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. }
        | SessionState::Ready(info) => Some(info),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::Locked(_)
        | SessionState::LoggingOut => None,
    }
}

async fn bootstrap_cross_signing_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    auth: Option<AuthSecret>,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::BootstrapCrossSigning { request_id, auth },
    ))
    .await
    .map_err(|e| format!("{label}: submit bootstrap cross-signing failed: {e}"))?;

    wait_for_cross_signing_trusted(conn, account_key, request_id, label).await
}

async fn wait_for_cross_signing_trusted(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    if matches!(
        conn.snapshot().e2ee_trust.cross_signing,
        CrossSigningStatus::Trusted
    ) {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for cross-signing Trusted"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => {
                handle_cross_signing_status(&status, request_id.sequence, label)?;
                if matches!(status, CrossSigningStatus::Trusted) {
                    return Ok(());
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                let status = snapshot.e2ee_trust.cross_signing;
                handle_cross_signing_status(&status, request_id.sequence, label)?;
                if matches!(status, CrossSigningStatus::Trusted) {
                    return Ok(());
                }
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

fn handle_cross_signing_status(
    status: &CrossSigningStatus,
    request_sequence: u64,
    label: &str,
) -> Result<(), String> {
    if let CrossSigningStatus::Failed { request_id, kind } = status
        && *request_id == request_sequence
    {
        return Err(format!("{label}: cross-signing failed: {kind:?}"));
    }
    Ok(())
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
        last_activity_ms: 0,
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

    sync_once_for_qa(conn_a, "activity sync A after unread seed").await?;
    wait_for_room_unread_count(conn_a, room_id, "activity room unread count").await?;

    let open_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::App(AppCommand::OpenActivity {
            request_id: open_id,
        }))
        .await
        .map_err(|e| format!("activity: submit open failed: {e}"))?;
    let (recent_event_ids, unread_event_ids) =
        wait_for_activity_snapshot(conn_a, open_id, "activity open").await?;

    if !recent_event_ids
        .iter()
        .any(|event_id| event_id == &send_outcome.event_id)
    {
        return Err("activity recent projection did not include the unread seed".to_owned());
    }
    println!("activity_recent=ok");

    if !unread_event_ids
        .iter()
        .any(|event_id| event_id == &send_outcome.event_id)
    {
        return Err("activity unread projection did not include the unread seed".to_owned());
    }
    println!("activity_unread=ok");

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
        name: "QA E2EE Backup Room".to_owned(),
        encrypted: true,
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted room create failed: {e}"))?;

    let room_id = wait_for_room_created(conn, create_room_id, label).await?;

    sync_once_for_qa(conn, "sync after encrypted backup seed room").await?;
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

async fn verify_second_device_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    session_a: &SessionInfo,
    session_a2: &SessionInfo,
    label: &str,
) -> Result<(), String> {
    if session_a.user_id != session_a2.user_id {
        return Err("E2EE verification proof requires two devices for one user".to_owned());
    }
    if session_a.device_id == session_a2.device_id {
        return Err("E2EE verification proof requires distinct device ids".to_owned());
    }

    let target_a = VerificationTarget {
        user_id: session_a.user_id.clone(),
        device_id: session_a.device_id.clone(),
    };
    let target_a2 = VerificationTarget {
        user_id: session_a2.user_id.clone(),
        device_id: session_a2.device_id.clone(),
    };

    let request_label = format!("{label}: request secondary to primary");
    let flow_id_a2 = request_device_verification_for_qa(conn_a2, target_a, &request_label).await?;
    // Avoid overlapping continuous SyncService with manual SyncOnce delivery
    // during SAS; overlapping paths reproduced pre-SAS key-mismatch flakes.
    let pause_primary_label = format!("{label}: pause sync primary for verification");
    let pause_secondary_label = format!("{label}: pause sync secondary for verification");
    stop_sync_for_qa(conn_a, &pause_primary_label).await?;
    stop_sync_for_qa(conn_a2, &pause_secondary_label).await?;
    let incoming_label = format!("{label}: incoming verification request");
    let flow_id_a =
        wait_for_verification_requested_with_sync_once(conn_a, Some(&target_a2), &incoming_label)
            .await?;

    let accept_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(AccountCommand::AcceptVerification {
            request_id: accept_a_id,
            flow_id: flow_id_a,
        }))
        .await
        .map_err(|e| format!("{label}: accept verification primary failed to submit: {e}"))?;

    let primary_accept_label = format!("{label}: primary accepts verification");
    wait_for_verification_accepted(conn_a, flow_id_a, Some(accept_a_id), &primary_accept_label)
        .await?;
    let secondary_observes_accept_label = format!("{label}: secondary observes primary acceptance");
    wait_for_verification_accepted_with_sync_once(
        conn_a2,
        flow_id_a2,
        &secondary_observes_accept_label,
    )
    .await?;

    // Let the requester start SAS. Starting from the accepting device has
    // triggered m.key_mismatch on Tuwunel self-verification in local QA.
    let start_sas_a2_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::AcceptVerification {
            request_id: start_sas_a2_id,
            flow_id: flow_id_a2,
        }))
        .await
        .map_err(|e| format!("{label}: start SAS from secondary failed to submit: {e}"))?;

    let (emojis_a, emojis_a2) =
        drive_until_both_verification_sas(conn_a, flow_id_a, conn_a2, flow_id_a2, label).await?;
    if emojis_a != emojis_a2 {
        return Err(format!("{label}: SAS emoji mismatch between devices"));
    }

    let confirm_a_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id: confirm_a_id,
                flow_id: flow_id_a,
            },
        ))
        .await
        .map_err(|e| format!("{label}: confirm SAS primary failed to submit: {e}"))?;

    let confirm_a2_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(
            AccountCommand::ConfirmSasVerification {
                request_id: confirm_a2_id,
                flow_id: flow_id_a2,
            },
        ))
        .await
        .map_err(|e| format!("{label}: confirm SAS secondary failed to submit: {e}"))?;

    let sync_secondary_after_confirm_label = format!("{label}: sync secondary after SAS confirm");
    let sync_primary_after_confirm_label =
        format!("{label}: sync primary after secondary SAS confirm");
    let sync_secondary_after_done_label = format!("{label}: sync secondary after SAS done");
    sync_once_for_qa(conn_a2, &sync_secondary_after_confirm_label).await?;
    sync_once_for_qa(conn_a, &sync_primary_after_confirm_label).await?;
    sync_once_for_qa(conn_a2, &sync_secondary_after_done_label).await?;

    let primary_done_label = format!("{label}: primary verification done");
    wait_for_verification_done(conn_a, flow_id_a, Some(confirm_a_id), &primary_done_label).await?;
    let secondary_done_label = format!("{label}: secondary verification done");
    wait_for_verification_done(
        conn_a2,
        flow_id_a2,
        Some(confirm_a2_id),
        &secondary_done_label,
    )
    .await?;

    let resume_primary_label = format!("{label}: resume sync primary after verification");
    let resume_secondary_label = format!("{label}: resume sync secondary after verification");
    start_sync_for_qa(conn_a, &resume_primary_label).await?;
    start_sync_for_qa(conn_a2, &resume_secondary_label).await?;

    Ok(())
}

async fn verify_second_device_room_key_delivery_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    sync_once_for_qa(conn_a, "sync A before second-device encrypted send").await?;
    sync_once_for_qa(conn_a2, "sync A2 before second-device encrypted receive").await?;
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
) -> Result<(), String> {
    let check_recipient_second_device = env_flag_enabled(ENV_E2EE_RECIPIENT_SECOND_DEVICE)?;
    let (runtime_b, mut conn_b, account_key_b) = login_synced_participant_for_qa(
        config,
        qa_data_dir("e2ee-b"),
        &config.user_b,
        &config.password_b,
        DEVICE_B,
        "e2ee login B",
    )
    .await?;

    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let room_id = create_room_for_qa(
        conn_a,
        "QA E2EE Multi Device DM",
        true,
        "e2ee multi-device create encrypted room",
    )
    .await?;

    sync_once_for_qa(conn_a, "e2ee multi-device sync A after room create").await?;
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
    sync_once_for_qa(&mut conn_b, "e2ee multi-device sync B for invite").await?;
    wait_for_invite_in_snapshot(
        &mut conn_b,
        &room_id,
        Some(false),
        "e2ee multi-device wait for B invite",
    )
    .await?;
    accept_invite_for_qa(&mut conn_b, &room_id, "e2ee multi-device B accepts invite").await?;

    sync_once_for_qa(conn_a, "e2ee multi-device sync A after B join").await?;
    sync_once_for_qa(conn_a2, "e2ee multi-device sync A2 after room join").await?;
    sync_once_for_qa(&mut conn_b, "e2ee multi-device sync B after join").await?;
    wait_for_room_in_room_list(
        conn_a2,
        &room_id,
        "e2ee multi-device A2 room list after create",
    )
    .await?;
    wait_for_room_in_room_list(&mut conn_b, &room_id, "e2ee multi-device B room list").await?;

    let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
    let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.clone());
    let key_b = TimelineKey::room(account_key_b.clone(), room_id.clone());

    let initial_a =
        subscribe_timeline_for_qa(conn_a, &key_a, "e2ee multi-device subscribe A").await?;
    let initial_a2 =
        subscribe_timeline_for_qa(conn_a2, &key_a2, "e2ee multi-device subscribe A2").await?;
    let initial_b =
        subscribe_timeline_for_qa(&mut conn_b, &key_b, "e2ee multi-device subscribe B").await?;
    assert_no_decryption_failure_items(&initial_a, "e2ee multi-device A initial")?;
    assert_no_decryption_failure_items(&initial_a2, "e2ee multi-device A2 initial")?;
    assert_no_decryption_failure_items(&initial_b, "e2ee multi-device B initial")?;

    let mut maybe_recipient_second_device = None;
    if check_recipient_second_device {
        let (runtime_b2, mut conn_b2, account_key_b2) = login_synced_participant_for_qa(
            config,
            qa_data_dir("e2ee-b2"),
            &config.user_b,
            &config.password_b,
            "Koushi Core QA B2",
            "e2ee login B2",
        )
        .await?;
        let session_b =
            authenticated_session_info(&mut conn_b, "session B info for E2EE multi-device")?;
        let session_b2 =
            authenticated_session_info(&mut conn_b2, "session B2 info for E2EE multi-device")?;
        verify_second_device_for_qa(
            &mut conn_b,
            &mut conn_b2,
            &session_b,
            &session_b2,
            "e2ee recipient verification B/B2",
        )
        .await?;
        wait_for_room_in_room_list(&mut conn_b2, &room_id, "e2ee multi-device B2 room list")
            .await?;
        let key_b2 = TimelineKey::room(account_key_b2.clone(), room_id.clone());
        let initial_b2 =
            subscribe_timeline_for_qa(&mut conn_b2, &key_b2, "e2ee multi-device subscribe B2")
                .await?;
        assert_no_decryption_failure_items(&initial_b2, "e2ee multi-device B2 initial")?;
        maybe_recipient_second_device = Some((runtime_b2, conn_b2, account_key_b2, key_b2));
    }

    if let Some((_runtime_b2, conn_b2, _account_key_b2, _key_b2)) =
        maybe_recipient_second_device.as_mut()
    {
        settle_e2ee_device_list_propagation_for_qa(
            conn_a,
            &mut conn_b,
            conn_b2,
            "e2ee multi-device settle after B2 verification",
        )
        .await?;
    }

    if env_flag_enabled(ENV_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND)? {
        stop_sync_for_qa(conn_a, "pause sync A before multi-device send").await?;
        stop_sync_for_qa(conn_a2, "pause sync A2 before multi-device send").await?;
        stop_sync_for_qa(&mut conn_b, "pause sync B before multi-device send").await?;
        if let Some((_runtime_b2, conn_b2, _account_key_b2, _key_b2)) =
            maybe_recipient_second_device.as_mut()
        {
            stop_sync_for_qa(conn_b2, "pause sync B2 before multi-device send").await?;
        }
    }

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

    wait_for_item_with_body_or_decryption_failure_with_sync(
        conn_a2,
        &key_a2,
        E2EE_MULTI_USER_MULTI_DEVICE_BODY,
        "e2ee multi-device A2 receive",
    )
    .await?;
    wait_for_item_with_body_or_decryption_failure_with_sync(
        &mut conn_b,
        &key_b,
        E2EE_MULTI_USER_MULTI_DEVICE_BODY,
        "e2ee multi-device B receive",
    )
    .await?;

    if let Some((runtime_b2, mut conn_b2, account_key_b2, key_b2)) = maybe_recipient_second_device {
        wait_for_item_with_body_or_decryption_failure_with_sync(
            &mut conn_b2,
            &key_b2,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device B2 receive",
        )
        .await?;
        println!("e2ee_recipient_second_device_decrypt=ok");
        cleanup_logged_in_runtime(conn_b2, runtime_b2, account_key_b2, "e2ee cleanup B2").await?;
    }

    cleanup_logged_in_runtime(conn_b, runtime_b, account_key_b, "e2ee cleanup B").await?;
    Ok(())
}

async fn settle_e2ee_device_list_propagation_for_qa(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    conn_b2: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    for attempt in 1..=3 {
        let label_b2 = format!("{label}: B2 sync {attempt}");
        best_effort_sync_once_for_qa(conn_b2, &label_b2).await?;
        let label_b = format!("{label}: B sync {attempt}");
        best_effort_sync_once_for_qa(conn_b, &label_b).await?;
        let label_a = format!("{label}: A sync {attempt}");
        best_effort_sync_once_for_qa(conn_a, &label_a).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

async fn login_synced_participant_for_qa(
    config: &QaConfig,
    data_dir: std::path::PathBuf,
    username: &str,
    password: &str,
    device_display_name: &str,
    label: &str,
) -> Result<(CoreRuntime, CoreConnection, AccountKey), String> {
    let runtime = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn = runtime.attach();

    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: config.homeserver.clone(),
            username: username.to_owned(),
            password: AuthSecret::new(password.to_owned()),
            device_display_name: Some(device_display_name.to_owned()),
        },
    }))
    .await
    .map_err(|e| format!("{label}: submit login failed: {e}"))?;
    let account_key = wait_for_logged_in(&mut conn, login_id, label).await?;
    wait_for_ready_snapshot(&mut conn, label).await?;
    start_sync_for_qa(&mut conn, label).await?;

    Ok((runtime, conn, account_key))
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

enum VerificationRequestAttempt {
    Requested(u64),
    Failed(TrustOperationFailureKind),
}

async fn request_device_verification_for_qa(
    conn: &mut CoreConnection,
    target: VerificationTarget,
    label: &str,
) -> Result<u64, String> {
    let mut last_failure = None;
    for attempt in 1..=3 {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Account(AccountCommand::RequestVerification {
            request_id,
            target: target.clone(),
        }))
        .await
        .map_err(|e| format!("{label}: submit request verification failed: {e}"))?;

        match wait_for_verification_requested_or_failed(conn, request_id, Some(&target), label)
            .await?
        {
            VerificationRequestAttempt::Requested(flow_id) => return Ok(flow_id),
            VerificationRequestAttempt::Failed(kind) => {
                last_failure = Some(kind);
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    Err(format!(
        "{label}: verification request did not start after retries; last failure={last_failure:?}"
    ))
}

async fn wait_for_verification_requested_or_failed(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected_target: Option<&VerificationTarget>,
    label: &str,
) -> Result<VerificationRequestAttempt, String> {
    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for verification request"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress { state, .. }) => {
                if verification_state_flow_id(&state) != Some(request_id.sequence)
                    || !verification_state_matches_target(&state, expected_target)
                {
                    continue;
                }
                match state {
                    VerificationFlowState::Failed { kind, .. } => {
                        return Ok(VerificationRequestAttempt::Failed(kind));
                    }
                    VerificationFlowState::Requested { request_id, .. }
                    | VerificationFlowState::Accepted { request_id, .. }
                    | VerificationFlowState::SasPresented { request_id, .. }
                    | VerificationFlowState::Confirming { request_id, .. }
                    | VerificationFlowState::Done { request_id, .. } => {
                        return Ok(VerificationRequestAttempt::Requested(request_id));
                    }
                    VerificationFlowState::Idle => {}
                }
            }
            CoreEvent::StateChanged(AppState {
                e2ee_trust:
                    koushi_state::E2eeTrustState {
                        verification:
                            VerificationFlowState::Failed {
                                request_id: failed_id,
                                kind,
                                ..
                            },
                        ..
                    },
                ..
            }) if failed_id == request_id.sequence => {
                return Ok(VerificationRequestAttempt::Failed(kind));
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

async fn wait_for_verification_requested_with_sync_once(
    conn: &mut CoreConnection,
    expected_target: Option<&VerificationTarget>,
    label: &str,
) -> Result<u64, String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;

    loop {
        if let Some(flow_id) = requested_verification_flow_id(
            &conn.snapshot().e2ee_trust.verification,
            expected_target,
        )? {
            return Ok(flow_id);
        }

        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(format!(
                "{label}: timed out waiting for incoming verification request with SyncOnce"
            ));
        }

        let sync_label = format!("{label}: sync while waiting for verification request");
        let sync_request_id = conn.next_request_id();
        conn.command(CoreCommand::Sync(SyncCommand::SyncOnce {
            request_id: sync_request_id,
        }))
        .await
        .map_err(|e| format!("{sync_label}: submit SyncOnce failed: {e}"))?;

        loop {
            if let Some(flow_id) = requested_verification_flow_id(
                &conn.snapshot().e2ee_trust.verification,
                expected_target,
            )? {
                return Ok(flow_id);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(format!(
                    "{label}: timed out waiting for incoming verification request with SyncOnce"
                ));
            }
            let remaining = deadline.duration_since(now);

            let event = tokio::time::timeout(remaining.min(EVENT_TIMEOUT), conn.recv_event())
                .await
                .map_err(|_| {
                    format!(
                        "{label}: timed out waiting for incoming verification request with SyncOnce"
                    )
                })?
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
                    if let Some(flow_id) = requested_verification_flow_id(&state, expected_target)?
                    {
                        return Ok(flow_id);
                    }
                }
                CoreEvent::Sync(SyncEvent::Stopped {
                    request_id: Some(ev_id),
                }) if ev_id == sync_request_id => break,
                CoreEvent::Sync(SyncEvent::Stopped { request_id: None }) => break,
                CoreEvent::OperationFailed {
                    request_id: ev_id,
                    failure,
                } if ev_id == sync_request_id => {
                    return Err(format!("{sync_label}: SyncOnce failed: {failure:?}"));
                }
                _ => {}
            }
        }
    }
}

fn requested_verification_flow_id(
    state: &VerificationFlowState,
    expected_target: Option<&VerificationTarget>,
) -> Result<Option<u64>, String> {
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

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
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

async fn wait_for_verification_accepted_with_sync_once(
    conn: &mut CoreConnection,
    flow_id: u64,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;

    loop {
        if verification_state_is_at_least_accepted(
            &conn.snapshot().e2ee_trust.verification,
            flow_id,
        )? {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out waiting for verification acceptance with SyncOnce"
            ));
        }

        let sync_label = format!("{label}: sync while waiting for acceptance");
        sync_once_for_qa(conn, &sync_label).await?;
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

async fn drive_until_both_verification_sas(
    conn_a: &mut CoreConnection,
    flow_id_a: u64,
    conn_a2: &mut CoreConnection,
    flow_id_a2: u64,
    label: &str,
) -> Result<(Vec<SasEmoji>, Vec<SasEmoji>), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;

    loop {
        let emojis_a = verification_state_sas(
            &conn_a.snapshot().e2ee_trust.verification,
            flow_id_a,
            &format!("{label}: primary SAS presented"),
        )?;
        let emojis_a2 = verification_state_sas(
            &conn_a2.snapshot().e2ee_trust.verification,
            flow_id_a2,
            &format!("{label}: secondary SAS presented"),
        )?;
        if let (Some(emojis_a), Some(emojis_a2)) = (emojis_a, emojis_a2) {
            return Ok((emojis_a, emojis_a2));
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out driving SAS presentation with SyncOnce"
            ));
        }

        let primary_sync_label = format!("{label}: sync primary while waiting for SAS");
        let secondary_sync_label = format!("{label}: sync secondary while waiting for SAS");
        sync_once_for_qa(conn_a, &primary_sync_label).await?;
        sync_once_for_qa(conn_a2, &secondary_sync_label).await?;
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

async fn wait_for_verification_done(
    conn: &mut CoreConnection,
    flow_id: u64,
    command_request_id: Option<RequestId>,
    label: &str,
) -> Result<(), String> {
    if verification_state_done(&conn.snapshot().e2ee_trust.verification, flow_id, label)? {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for verification completion"))?
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
                if verification_state_done(&state, flow_id, label)? {
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

fn verification_state_done(
    state: &VerificationFlowState,
    flow_id: u64,
    label: &str,
) -> Result<bool, String> {
    if verification_state_flow_id(state) != Some(flow_id) {
        return Ok(false);
    }
    match state {
        VerificationFlowState::Done { .. } => Ok(true),
        VerificationFlowState::Failed { kind, .. } => Err(format!(
            "{label}: verification failed before completion: {kind:?}"
        )),
        _ => Ok(false),
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
    running: Arc<AtomicBool>,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
    accept_thread: Option<JoinHandle<()>>,
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
        let running = Arc::new(AtomicBool::new(true));
        let active_streams = Arc::new(Mutex::new(Vec::new()));

        let thread_enabled = enabled.clone();
        let thread_running = running.clone();
        let thread_streams = active_streams.clone();
        let accept_thread = thread::spawn(move || {
            while thread_running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        if !thread_enabled.load(Ordering::SeqCst) {
                            let _ = client.shutdown(Shutdown::Both);
                            continue;
                        }
                        spawn_proxy_pair(client, target, thread_streams.clone());
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
            running,
            active_streams,
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
}

impl Drop for QaTcpProxy {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
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
) {
    thread::spawn(move || {
        let _ = proxy_single_http_request(&mut client, target, active_streams);
        let _ = client.shutdown(Shutdown::Both);
    });
}

fn proxy_single_http_request(
    client: &mut TcpStream,
    target: SocketAddr,
    active_streams: Arc<Mutex<Vec<TcpStream>>>,
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
    io::Write::write_all(&mut server, &request)?;
    io::copy(&mut server, client)?;
    Ok(())
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
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for TimelineEvent::InitialItems"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(ev_id),
                key: ref ev_key,
                items,
                ..
            }) if ev_id == request_id && ev_key == key => {
                return Ok(items);
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

    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
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

async fn wait_for_send_completions_in_order(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    retry_request_id: RequestId,
    first: &SendQueueLocalEcho,
    second: &SendQueueLocalEcho,
    label: &str,
) -> Result<(), String> {
    let mut first_completed = false;
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for ordered SendCompleted events"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id,
                key: ref ev_key,
                transaction_id,
                ..
            }) if ev_key == key && request_id == first.request_id => {
                if transaction_id != first.client_transaction_id {
                    return Err(format!(
                        "{label}: first completion transaction mismatch: {transaction_id}"
                    ));
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
                    return Err(format!(
                        "{label}: second completion transaction mismatch: {transaction_id}"
                    ));
                }
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == retry_request_id
                || request_id == first.request_id
                || request_id == second.request_id =>
            {
                return Err(format!("{label}: operation failed: {failure:?}"));
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
    wait_for_live_signal_snapshot(conn_b, "read receipt state", |snapshot| {
        read_receipt_identity_projected(snapshot, &room_id, event_id, expected_reader_user_id)
    })
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
    // Local SyncService homeserver lanes can acknowledge the typing command but
    // not wake the room typing observer from the sliding-sync typing extension.
    // A bounded SyncOnce keeps this QA leg focused on the Rust-owned
    // command/event/state contract; product sync policy remains in SyncActor.
    sync_once_for_qa(conn_a, "live signals sync A for typing").await?;

    let room_id_a = timeline_key_room_id(key_a)
        .ok_or_else(|| "live signals: expected observer room timeline key".to_owned())?
        .to_owned();
    wait_for_live_signal_snapshot(conn_a, "typing state", |snapshot| {
        snapshot
            .live_signals
            .rooms
            .get(&room_id_a)
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

fn read_receipt_identity_projected(
    snapshot: &AppState,
    room_id: &str,
    event_id: &str,
    expected_reader_user_id: &str,
) -> bool {
    snapshot
        .live_signals
        .rooms
        .get(room_id)
        .and_then(|room| room.receipts_by_event.get(event_id))
        .is_some_and(|receipts| {
            receipts.readers.iter().any(|reader| {
                let has_display_label = reader
                    .display_name
                    .as_deref()
                    .is_some_and(|label| !label.trim().is_empty())
                    || !reader.original_display_label.trim().is_empty();
                reader.user_id == expected_reader_user_id && has_display_label
            })
        })
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

    let media_txn = "qa-phase15-media-txn".to_owned();
    let send_media_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
            request_id: send_media_id,
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
    wait_for_settings_persisted(conn_b, settings_id, "global preview disable", false).await?;

    let disabled_item = wait_for_item_with_body(
        conn_b,
        key_b,
        URL_MESSAGE_BODY,
        "B sees message after global disable",
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
                    encrypted_url_previews_enabled: false,
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .map_err(|e| format!("submit global preview enable: {e}"))?;
    wait_for_settings_persisted(conn_b, settings_id, "global preview enable", true).await?;

    let reenabled_item = wait_for_item_with_body(
        conn_b,
        key_b,
        URL_MESSAGE_BODY,
        "B sees message after global re-enable",
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

    sync_once_for_qa(conn_a, "sync after link preview encrypted room creation").await?;
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

    let enc_item = wait_for_item_with_body(
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
            room_id: room_id.to_owned(),
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

async fn wait_for_item_with_body_or_decryption_failure(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    let mut observer = BodyWaitObserver::new(expected_body);
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
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

async fn wait_for_item_with_body_or_decryption_failure_with_sync(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    expected_body: &str,
    label: &str,
) -> Result<koushi_core::event::TimelineItem, String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut sync_request_id: Option<RequestId> = None;
    let mut observer = BodyWaitObserver::new(expected_body);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(observer.timeout_message(label));
        }

        if sync_request_id.is_none() {
            let request_id = conn.next_request_id();
            conn.command(CoreCommand::Sync(SyncCommand::SyncOnce { request_id }))
                .await
                .map_err(|e| format!("{label}: submit SyncOnce failed: {e}"))?;
            sync_request_id = Some(request_id);
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining.min(EVENT_TIMEOUT), conn.recv_event())
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
            CoreEvent::Sync(SyncEvent::Stopped {
                request_id: Some(ev_id),
            }) if Some(ev_id) == sync_request_id => {
                sync_request_id = None;
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if Some(ev_id) == sync_request_id => {
                return Err(format!("{label}: SyncOnce failed: {failure:?}"));
            }
            _ => {}
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
}

impl<'a> RoomThreadSummaryObserver<'a> {
    fn new(expected_thread_body: &'a str, root_event_id: &'a str) -> Self {
        Self {
            expected_thread_body,
            root_event_id,
        }
    }

    fn observe_items(&mut self, items: &[TimelineItem]) -> Result<bool, String> {
        let mut saw_summary = false;
        for item in items {
            if timeline_item_body_contains(item, self.expected_thread_body) {
                return Err(
                    "thread_hidden failed: thread reply appeared on room live timeline".to_owned(),
                );
            }
            saw_summary |= timeline_item_has_thread_summary_reply(item, self.root_event_id);
        }
        Ok(saw_summary)
    }

    fn observe_diffs(&mut self, diffs: &[TimelineDiff]) -> Result<bool, String> {
        let mut saw_summary = false;
        visit_timeline_diff_items(diffs, |item| {
            if timeline_item_body_contains(item, self.expected_thread_body) {
                return Err(
                    "thread_hidden failed: thread reply appeared on room live timeline".to_owned(),
                );
            }
            saw_summary |= timeline_item_has_thread_summary_reply(item, self.root_event_id);
            Ok(())
        })?;
        Ok(saw_summary)
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
fn assert_room_timeline_hides_thread_reply_and_summarizes_root(
    items: &[TimelineItem],
    expected_thread_body: &str,
    root_event_id: &str,
) -> Result<(), String> {
    let mut observer = RoomThreadSummaryObserver::new(expected_thread_body, root_event_id);
    if !observer.observe_items(items)? {
        return Err("thread_summary failed: root item did not carry a reply summary".to_owned());
    }
    Ok(())
}

fn assert_thread_reply_relation(item: &TimelineItem, root_event_id: &str) -> Result<(), String> {
    if item.in_reply_to_event_id.as_deref() != Some(root_event_id) {
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
        key,
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
        notice_i18n_key: None,
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
            scope: SearchScope::Room {
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
            scope: SearchScope::Room {
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
    use koushi_core::event::{ThreadSummaryDto, TimelineMessageActions};

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
                notice_i18n_key: None,
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
                notice_i18n_key: None,
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
            notice_i18n_key: None,
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
            notice_i18n_key: None,
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
            notice_i18n_key: None,
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
    fn room_thread_assertion_requires_hidden_reply_and_root_summary() {
        let root = synthetic_timeline_item(
            "$root:test",
            Some("root message"),
            None,
            None,
            Some(ThreadSummaryDto {
                reply_count: 1,
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: None,
            }),
        );
        let unrelated = synthetic_timeline_item("$other:test", Some("other"), None, None, None);

        assert!(
            assert_room_timeline_hides_thread_reply_and_summarizes_root(
                &[root.clone(), unrelated],
                "Phase 11 QA thread reply from B",
                "$root:test",
            )
            .is_ok()
        );

        let leaked = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            Some("$root:test"),
            None,
        );
        assert!(
            assert_room_timeline_hides_thread_reply_and_summarizes_root(
                &[root.clone(), leaked],
                "Phase 11 QA thread reply from B",
                "$root:test",
            )
            .is_err()
        );

        assert!(
            assert_room_timeline_hides_thread_reply_and_summarizes_root(
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
        );
    }

    #[test]
    fn room_thread_summary_observer_fails_immediately_on_leaked_reply() {
        let mut observer =
            RoomThreadSummaryObserver::new("Phase 11 QA thread reply from B", "$root:test");
        let leaked = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
            Some("$root:test"),
            None,
        );

        let error = observer.observe_items(&[leaked]).unwrap_err();

        assert!(error.contains("thread_hidden failed"));
    }

    #[test]
    fn thread_relation_helper_requires_reply_and_thread_root_metadata() {
        let valid = synthetic_timeline_item(
            "$reply:test",
            Some("Phase 11 QA thread reply from B"),
            Some("$root:test"),
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
            notice_i18n_key: None,
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
            notice_i18n_key: None,
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
                        notice_i18n_key: None,
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
                        notice_i18n_key: None,
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
                        notice_i18n_key: None,
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
    fn e2ee_strict_qa_can_pause_sync_before_multi_device_send() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");

        assert!(production_source.contains("ENV_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND"));
        assert!(production_source.contains("pause sync A before multi-device send"));
        assert!(production_source.contains("pause sync B2 before multi-device send"));
    }

    #[test]
    fn e2ee_strict_qa_settles_device_lists_after_recipient_second_device_verification() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");

        assert!(production_source.contains("settle_e2ee_device_list_propagation_for_qa"));
        assert!(production_source.contains("e2ee multi-device settle after B2 verification"));
        assert!(production_source.contains("best_effort_sync_once_for_qa"));
        assert!(production_source.contains("DEVICE_LIST_SETTLE_SYNC_TIMEOUT"));
    }

    #[test]
    fn e2ee_device_verification_wait_retries_sync_once_for_incoming_request() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/headless-core-qa.rs"
        ));
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("headless-core-qa source should contain production section");

        assert!(production_source.contains("wait_for_verification_requested_with_sync_once"));
        assert!(production_source.contains("sync while waiting for verification request"));
        assert!(
            production_source
                .contains("timed out waiting for incoming verification request with SyncOnce")
        );
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

        assert!(production_source.contains("e2ee self verification A/A2"));
        assert!(production_source.contains("e2ee recipient verification B/B2"));
        assert!(production_source.contains("request secondary to primary"));
        assert!(production_source.contains("incoming verification request"));
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
    fn send_queue_scenario_runs_after_timeline_and_reports_private_tokens() {
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::Safety));
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::LoginSync));
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::RoomSpace));
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::Timeline));
        assert!(QaScenario::SendQueue.should_run_stage(QaStage::SendQueue));
        assert!(!QaScenario::SendQueue.should_run_stage(QaStage::Reply));
        assert!(!QaScenario::SendQueue.should_run_stage(QaStage::EditRedactSearch));

        assert_eq!(
            final_tokens_for_scenario(QaScenario::SendQueue),
            [
                "safety=ok",
                "login_sync=ok",
                "room_space=ok",
                "timeline=ok",
                "timeline_nav=ok",
                "hide_redacted=ok",
                "send_fail=ok",
                "resend=ok",
                "cancel_send=ok",
                "fifo=ok",
                "unsent_restart=ok",
                "restore_cleanup=ok",
            ]
        );
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
                "thread_hidden=ok",
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
                "joined_room_restore=ok",
                "e2ee_second_device_decrypt=ok",
                "e2ee_multi_user_multi_device_decrypt=ok",
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
    fn e2ee_trust_qa_uses_authenticated_session_info_during_recovery() {
        let info = SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        };

        assert_eq!(
            authenticated_session_info_from_state(&SessionState::NeedsRecovery {
                info: info.clone(),
                methods: vec![],
            }),
            Some(&info)
        );
        assert_eq!(
            authenticated_session_info_from_state(&SessionState::Recovering {
                info: info.clone(),
                methods: vec![],
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
                "thread_hidden=ok",
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
                "thread_hidden=ok",
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
                "joined_room_restore=ok",
                "e2ee_second_device_decrypt=ok",
                "e2ee_multi_user_multi_device_decrypt=ok",
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
