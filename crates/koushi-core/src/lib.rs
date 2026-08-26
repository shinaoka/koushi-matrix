#![recursion_limit = "256"]

//! Koushi core runtime.
//!
//! The only production runtime owner: actor lifecycle, command routing,
//! event emission, SDK session handles, background tasks, and AppState
//! projection live here, behind the `CoreCommand`/`CoreEvent` boundary.
//!
//! Normative architecture: `docs/architecture/overview.md`.
//! Migration spec: `docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md`.

pub mod account;
pub(crate) mod account_work;
mod activity_resolution;
pub(crate) mod cached_image;
mod causal_projection;
pub mod command;
pub mod composer_draft_lifecycle;
mod credential_vault;
mod direct_message_classification;
pub mod event;
pub mod executor;
pub mod failure;
pub mod ids;
pub mod link_preview;
mod live_catchup;
mod live_tail_freshness;
#[cfg(any(test, feature = "test-hooks"))]
pub mod login_store_test_support;
pub mod media_preparation;
pub(crate) mod mention_candidates;
pub(crate) mod read_state;
pub mod renderable_thumbnail;
pub mod room;
mod room_key_receive;
mod room_key_recovery;
#[cfg(feature = "test-hooks")]
pub mod room_subscription_residency_test_support;
pub mod runtime;
pub(crate) mod scheduled_send;
pub mod search;
pub(crate) mod search_crawler;
mod send_diagnostics;
pub mod settings;
mod sliding_sync_diagnostics;
pub(crate) mod startup_trace;
pub mod state_delta;
pub mod store;
pub mod sync;
pub mod threads_list;
pub mod timeline;
pub(crate) mod unread_trace;

pub use command::{
    AccountCommand, AppCommand, CoreCommand, CreateRoomOptions, CreateRoomParentSpace,
    CreateRoomVisibility, ImageUploadCompressionPolicy, ImageUploadCompressionState,
    ImageUploadDimensions, ImageUploadVariantInfo, ImageUploadVariantKind, KeyRequestOrigin,
    MediaDownloadSelection, RoomCommand, RoomKeyExportRequest, RoomKeyImportRequest, SearchCommand,
    SearchScope, SecureBackupPassphraseChangeRequest, SecureBackupSetupRequest, SetAvatarRequest,
    SyncCommand, TimelineCommand, UploadMediaKind, UploadMediaRequest, UploadMediaThumbnail,
};
pub use direct_message_classification::DirectAccountDataSource;
pub use event::{
    AccountEvent, ActivityEvent, AppStateSnapshot, CjkTextPolicyEvent, CoreEvent, E2eeTrustEvent,
    EncryptionDebugOperationOutcome, IntentNoOpReason, IntentOutcome, LinkPreview,
    LinkPreviewImage, LinkPreviewState, LocalEncryptionEvent, NativeAttentionEvent,
    PaginationDirection, PaginationState, ReactionGroup, ReactionSender, RoomEvent,
    RoomKeyReshareOutcome, SearchEvent, SearchResultItem, SyncEvent, TimelineDiff, TimelineEvent,
    TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId, TimelineMedia,
    TimelineMediaKind, TimelineMediaSource, TimelineMediaThumbnail, TimelineMessageKind,
    TimelineNavigationSnapshot, TimelineReadStateSync, TimelineResyncReason,
    TimelineSendFailureReason, TimelineSendState, TimelineSpoilerSpan, TimelineUnreadPosition,
    TimelineViewportObservation,
};
pub use failure::{
    CoreFailure, LoginFailureKind, ProfileFailureKind, ReadStateFailureKind, RecoveryFailureKind,
    RoomFailureKind, SearchFailureKind, SyncFailureKind, TimelineFailureKind,
};
pub use ids::{
    AccountKey, RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey,
    TimelineKind,
};
pub use koushi_state::MediaTransferProgress;
pub use runtime::{
    COMMAND_INBOX_CAPACITY, CommandSubmitError, CoreCommandHandle, CoreConnection, CoreRuntime,
    EVENT_QUEUE_CAPACITY, EventStreamLag,
};
pub use sliding_sync_diagnostics::{
    DiagnosticAgeBucket, SlidingSyncDiagnostics, SlidingSyncDiagnosticsSnapshot,
    SlidingSyncDiscoveryDiagnostic, SlidingSyncDiscoverySource, SlidingSyncDiscoveryState,
    SlidingSyncEngine, SlidingSyncFailureDiagnostic, SlidingSyncFailureKind,
    SlidingSyncFailureOrigin, SlidingSyncFailureRetryability, SlidingSyncFailureStage,
    SlidingSyncHttpErrorSource, SlidingSyncHttpStatus, SlidingSyncHttpStatusClass,
    SlidingSyncLifecycle, SlidingSyncMatrixErrorKind, SlidingSyncProvisionalHandoffBucket,
    SlidingSyncRequestSchema, SlidingSyncSdkVersion,
};
pub use state_delta::{StateDelta, StateDeltaChangedSlices, build_state_delta};
