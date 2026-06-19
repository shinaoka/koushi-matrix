//! Matrix Desktop core runtime.
//!
//! The only production runtime owner: actor lifecycle, command routing,
//! event emission, SDK session handles, background tasks, and AppState
//! projection live here, behind the `CoreCommand`/`CoreEvent` boundary.
//!
//! Normative architecture: `docs/architecture/overview.md`.
//! Migration spec: `docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md`.

pub mod account;
pub mod command;
pub mod event;
pub mod executor;
pub mod failure;
pub mod ids;
pub(crate) mod link_preview;
pub(crate) mod messages_backpressure;
pub mod room;
pub mod runtime;
pub(crate) mod scheduled_send;
pub mod search;
pub(crate) mod search_crawler;
pub mod settings;
pub mod store;
pub mod sync;
pub mod threads_list;
pub mod timeline;

pub use command::{
    AccountCommand, AppCommand, CoreCommand, ImageUploadCompressionPolicy,
    ImageUploadCompressionState, ImageUploadDimensions, ImageUploadVariantInfo,
    ImageUploadVariantKind, MediaDownloadSelection, RoomCommand, RoomKeyExportRequest,
    RoomKeyImportRequest, SearchCommand, SearchScope, SecureBackupPassphraseChangeRequest,
    SecureBackupSetupRequest, SetAvatarRequest, SyncCommand, TimelineCommand, UploadMediaKind,
    UploadMediaRequest, UploadMediaThumbnail,
};
pub use event::{
    AccountEvent, ActivityEvent, AppStateSnapshot, CjkTextPolicyEvent, CoreEvent, E2eeTrustEvent,
    LinkPreview, LinkPreviewImage, LinkPreviewState, LocalEncryptionEvent, NativeAttentionEvent,
    PaginationDirection, PaginationState, ReactionGroup, RoomEvent, SearchEvent, SearchResultItem,
    SyncBackendKind, SyncEvent, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId,
    TimelineMedia, TimelineMediaKind, TimelineMediaSource, TimelineMediaThumbnail,
    TimelineMessageKind, TimelineNavigationSnapshot, TimelineResyncReason,
    TimelineSendFailureReason, TimelineSendState, TimelineSpoilerSpan, TimelineUnreadPosition,
    TimelineViewportObservation,
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
    COMMAND_INBOX_CAPACITY, CommandSubmitError, CoreConnection, CoreRuntime, EVENT_QUEUE_CAPACITY,
    EventStreamLag,
};
