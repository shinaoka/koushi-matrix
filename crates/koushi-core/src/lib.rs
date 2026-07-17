//! Koushi core runtime.
//!
//! The only production runtime owner: actor lifecycle, command routing,
//! event emission, SDK session handles, background tasks, and AppState
//! projection live here, behind the `CoreCommand`/`CoreEvent` boundary.
//!
//! Normative architecture: `docs/architecture/overview.md`.
//! Migration spec: `docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md`.

pub mod account;
mod activity_resolution;
pub(crate) mod cached_image;
mod causal_projection;
pub mod command;
pub mod event;
pub mod executor;
pub mod failure;
pub mod ids;
pub mod link_preview;
mod live_catchup;
mod live_tail_freshness;
pub mod media_preparation;
pub(crate) mod messages_backpressure;
pub mod renderable_thumbnail;
pub mod room;
pub mod runtime;
pub(crate) mod scheduled_send;
pub mod search;
pub(crate) mod search_crawler;
pub mod settings;
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
    ImageUploadDimensions, ImageUploadVariantInfo, ImageUploadVariantKind, MediaDownloadSelection,
    RoomCommand, RoomKeyExportRequest, RoomKeyImportRequest, SearchCommand, SearchScope,
    SecureBackupPassphraseChangeRequest, SecureBackupSetupRequest, SetAvatarRequest, SyncCommand,
    TimelineCommand, UploadMediaKind, UploadMediaRequest, UploadMediaThumbnail,
};
pub use event::{
    AccountEvent, ActivityEvent, AppStateSnapshot, CjkTextPolicyEvent, CoreEvent, E2eeTrustEvent,
    IntentNoOpReason, IntentOutcome, LinkPreview, LinkPreviewImage, LinkPreviewState,
    LocalEncryptionEvent, NativeAttentionEvent, PaginationDirection, PaginationState,
    ReactionGroup, RoomEvent, SearchEvent, SearchResultItem, SyncBackendKind, SyncEvent,
    TimelineDiff, TimelineEvent, TimelineGapPosition, TimelineItem, TimelineItemId, TimelineMedia,
    TimelineMediaKind, TimelineMediaSource, TimelineMediaThumbnail, TimelineMessageKind,
    TimelineNavigationSnapshot, TimelineResyncReason, TimelineSendFailureReason, TimelineSendState,
    TimelineSpoilerSpan, TimelineUnreadPosition, TimelineViewportObservation,
};
pub use failure::{
    CoreFailure, LoginFailureKind, ProfileFailureKind, RecoveryFailureKind, RoomFailureKind,
    SearchFailureKind, SyncFailureKind, TimelineFailureKind,
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
pub use state_delta::{StateDelta, StateDeltaChangedSlices, build_state_delta};
