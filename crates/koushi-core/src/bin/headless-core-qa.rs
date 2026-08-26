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
//! Phase 4 flow (one required Simplified Sliding Sync runtime):
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
    Arc, Mutex,
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
use koushi_core::composer_draft_lifecycle::ComposerDraftScope;
use koushi_core::event::{
    AccountEvent, ActivityEvent, CoreEvent, E2eeTrustEvent, EncryptionDebugOperationOutcome,
    LinkPreviewState, LiveSignalsEvent, LocalEncryptionEvent, PaginationDirection, PaginationState,
    RoomEvent, SearchEvent, SyncEvent, TimelineAnchorRestoreStatus, TimelineDiff, TimelineEvent,
    TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId, TimelineMessageActions,
    TimelineReadStateSync, TimelineSendState, TimelineUnreadPosition, TimelineViewportObservation,
};
use koushi_core::failure::{CoreFailure, RoomFailureKind};
use koushi_core::ids::{AccountKey, RequestId, TimelineKey, TimelineKind};
use koushi_core::runtime::{CoreConnection, CoreRuntime, EventStreamLag};
use koushi_state::{
    ActivityMarkReadTarget, ActivityRowKind, ActivityState, AppAction, AppState, AuthSecret,
    ComposerDocument, ComposerKey, ComposerKeyEvent, ComposerKeyModifiers, ComposerResolvedAction,
    ComposerResolverContext, ComposerSelection, ComposerSendShortcut, ComposerSurface,
    ComposerTarget, CurrentSessionStatusState, CurrentSessionSyncState, DeviceCleanupLocalMode,
    DeviceCleanupState, DirectoryQuery, DirectoryRoomSummary, DisplaySettings,
    IdentityResetAuthRequest, IdentityResetAuthType, IdentityResetState,
    ImageUploadCompressionMode, KeyBackupStatus, LocalEncryptionHealth, LocalEncryptionState,
    MentionCandidatesCompleteness, MentionCandidatesTarget, MentionIntent, MentionSurface,
    MentionTarget, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionDispatchState, NativeAttentionObservationKind, NativeAttentionProjectionInput,
    NativeAttentionState, NativeAttentionSuppressionReason, OperationFailureKind, PresenceKind,
    RecoveryRequest, ReplyQuoteState, RoomAttentionKind, RoomListFilter,
    RoomManagementOperationKind, RoomManagementOperationState, RoomMentionPermission,
    RoomModerationAction, RoomNotificationMode, RoomSettingChange, RoomSettingsSnapshot,
    RoomSummary, RoomTags, SasEmoji, ScheduledSendCapability, SearchCrawlerFailureKind,
    SearchCrawlerRoomState, SearchCrawlerSettings, SearchCrawlerSpeed, SessionAuthenticationMethod,
    SessionInfo, SessionState, SessionStatusRefreshTrigger, SettingsPatch,
    SettingsPersistenceState, StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind,
    TimelineMediaGalleryItem, TimelineMediaGalleryMedia, TimelineMediaGallerySource,
    TimelineMediaKind, VerificationFlowState, VerificationTarget, build_formatted_message_draft,
    compose_sidebar, native_attention_state_from_rooms, reduce, resolve_composer_key_action,
};

#[path = "headless_core_qa/cleanup.rs"]
mod cleanup;
#[path = "headless_core_qa/diagnostics.rs"]
mod diagnostics;
#[path = "headless_core_qa/event_wait.rs"]
mod event_wait;
#[path = "headless_core_qa/fixtures.rs"]
mod fixtures;
#[path = "headless_core_qa/orchestrator.rs"]
mod orchestrator;
#[path = "headless_core_qa/participants.rs"]
mod participants;
#[path = "headless_core_qa/registry.rs"]
mod registry;
#[path = "headless_core_qa/scenarios/identity.rs"]
mod scenario_identity;
#[path = "headless_core_qa/scenarios/read_state.rs"]
mod scenario_read_state;
#[path = "headless_core_qa/scenarios/rooms.rs"]
mod scenario_rooms;
#[path = "headless_core_qa/scenarios/search.rs"]
mod scenario_search;
#[path = "headless_core_qa/scenarios/timeline.rs"]
mod scenario_timeline;

use orchestrator::run_async;
use registry::{
    QaConfig, QaScenario, assert_file_credential_store_active, scenario_preflight_error,
};

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

#[cfg(test)]
#[path = "headless_core_qa/contracts.rs"]
mod contracts;
#[cfg(test)]
#[path = "headless_core_qa/login_store_contracts.rs"]
mod login_store_contracts;
