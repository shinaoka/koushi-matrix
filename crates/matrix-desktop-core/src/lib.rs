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
pub mod runtime;
pub mod store;
pub mod sync;

pub use command::{
    AccountCommand, AppCommand, CoreCommand, RoomCommand, SearchCommand, SearchScope, SyncCommand,
    TimelineCommand,
};
pub use event::{
    AccountEvent, AppStateSnapshot, CoreEvent, PaginationDirection, PaginationState, RoomEvent,
    SearchEvent, SearchResultItem, SyncBackendKind, SyncEvent, TimelineDiff, TimelineEvent,
    TimelineItem, TimelineItemId, TimelineResyncReason,
};
pub use failure::{
    CoreFailure, LoginFailureKind, RecoveryFailureKind, RoomFailureKind, SearchFailureKind,
    SyncFailureKind, TimelineFailureKind,
};
pub use ids::{
    AccountKey, RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey,
    TimelineKind,
};
pub use runtime::{
    COMMAND_INBOX_CAPACITY, CommandSubmitError, CoreConnection, CoreRuntime, EVENT_QUEUE_CAPACITY,
    EventStreamLag,
};

#[cfg(test)]
mod tests;
