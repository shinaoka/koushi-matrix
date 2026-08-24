//! Public event boundary. Events carry the originating `RequestId` when one
//! exists; identifiers and visible bodies are allowed, secrets never.

use koushi_state::{AppState, SessionState};
use serde::{Deserialize, Serialize};

use crate::failure::CoreFailure;
use crate::ids::RequestId;
use crate::state_delta::StateDelta;

mod account;
mod attention;
mod live_signals;
mod room;
mod search;
mod timeline;

pub use account::{
    AccountEvent, E2eeTrustEvent, EventCacheFailureReasonClass, EventCacheSubscribeStatus,
    LocalEncryptionEvent,
};
pub use attention::{ActivityEvent, NativeAttentionEvent};
pub use live_signals::LiveSignalsEvent;
pub use room::{
    EncryptionDebugOperationOutcome, RoomEvent, RoomKeyReshareOutcome,
    project_room_event_display_labels,
};
pub use search::{SearchEvent, SearchResultItem};
pub use timeline::{
    CjkTextPolicyEvent, LinkPreview, LinkPreviewImage, LinkPreviewState, PaginationDirection,
    PaginationState, ReactionGroup, ReactionSender, RoomKeyRequestStage, RoomKeyRequestStateDto,
    RoomKeyRequestWithheldCode, ThreadRootProjectionDto, ThreadRootProjectionSourceDto,
    ThreadRootProjectionStateDto, ThreadSummaryDto, ThreadsListEvent, TimelineAnchorRestoreStatus,
    TimelineCodeBlock, TimelineDiff, TimelineDisplayLabelUpdate, TimelineEvent,
    TimelineFormattedBody, TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId,
    TimelineLinkRange, TimelineMedia, TimelineMediaKind, TimelineMediaSource,
    TimelineMediaThumbnail, TimelineMegolmSessionReason, TimelineMessageActions,
    TimelineMessageKind, TimelineMessageSource, TimelineNavigationSnapshot, TimelineNoticeI18n,
    TimelineNoticeI18nKey, TimelineReadStateSync, TimelineResyncReason, TimelineSendFailureReason,
    TimelineSendState, TimelineSpoilerSpan, TimelineUnableToDecrypt, TimelineUnableToDecryptReason,
    TimelineUnreadPosition, TimelineViewportObservation, derive_display_label_updates,
    derive_display_label_updates_for_user_ids, matrix_to_event_permalink,
    message_actions_for_timeline_item, message_source_for_timeline_item,
    project_timeline_event_display_labels, project_timeline_item_display_labels,
};

/// Serializable UI snapshot. The full timeline item lists never live here
/// (Async rule 4); timeline data flows as diffs.
pub type AppStateSnapshot = koushi_state::AppState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionedAppStateSnapshot {
    pub generation: u64,
    pub state: AppStateSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    Event,
    Room,
    User,
}

/// Reason a SelectRoom intent produced no state change.
///
/// `AlreadyActive` is a benign idempotent no-op (the room was already
/// selected). `SessionNotReady` and `RoomNotInState` are retryable failure
/// no-ops; the caller should surface a specific diagnostic rather than a
/// generic timeout.
///
/// Private-data-free: never carries room ids, user ids, or message bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentNoOpReason {
    /// The session was not in a ready state at reduce time.
    SessionNotReady,
    /// The targeted room was not present in `state.rooms` at reduce time.
    RoomNotInState,
    /// The room was already the active room (idempotent, not a failure).
    AlreadyActive,
    /// The focused projection settled without the requested event, so the
    /// navigation safely returned to the room's live timeline.
    TimelineTargetMissing,
}

/// Terminal outcome of a user-intent command (§4.7 Slice 1 telemetry-lane
/// event). Carried by `CoreEvent::IntentLifecycle`.
///
/// Slice 1 covers `SelectRoom` only. Future slices will extend this to
/// `SelectSpace`, send, pin/unpin, etc.
///
/// `BenignNoOp` means the intent was received but had no effect for a
/// harmless reason (e.g. already active). `FailedNoOp` means the intent
/// could not be applied and should be retried or surfaced as an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum IntentOutcome {
    /// The reducer applied the intent and state was mutated as expected.
    Committed,
    /// The intent had no effect for a harmless, idempotent reason.
    BenignNoOp(IntentNoOpReason),
    /// The intent could not be applied; the caller should surface this as an
    /// error rather than a silent timeout.
    FailedNoOp(IntentNoOpReason),
}

#[derive(Clone, Debug)]
pub enum CoreEvent {
    StateDelta(StateDelta),
    StateChanged(AppStateSnapshot),
    Account(AccountEvent),
    Sync(SyncEvent),
    Room(RoomEvent),
    Timeline(TimelineEvent),
    LiveSignals(LiveSignalsEvent),
    Search(SearchEvent),
    E2eeTrust(E2eeTrustEvent),
    Activity(ActivityEvent),
    LocalEncryption(LocalEncryptionEvent),
    NativeAttention(NativeAttentionEvent),
    CjkTextPolicy(CjkTextPolicyEvent),
    ThreadsList(ThreadsListEvent),
    OperationFailed {
        request_id: RequestId,
        failure: CoreFailure,
    },
    /// Telemetry-lane event: the terminal outcome of a user-intent command.
    ///
    /// This event is on a DEDICATED TELEMETRY LANE — it must never be mixed
    /// into product `StateDelta` or `StateChanged`, and product state must
    /// never be derived from it. It is emitted after the reducer runs so the
    /// AppActor can correlate the outcome with the originating `request_id`.
    ///
    /// Slice 1 covers `SelectRoom` only.
    IntentLifecycle {
        request_id: RequestId,
        outcome: IntentOutcome,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SyncEvent {
    Started { request_id: Option<RequestId> },
    Running,
    Reconnecting,
    Failed,
    Stopped { request_id: Option<RequestId> },
}

pub fn timeline_projection_own_user_id(state: &AppState) -> Option<&str> {
    match &state.session {
        SessionState::Ready(info) => Some(info.user_id.as_str()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::Authenticating { .. }
        | SessionState::Provisional { .. }
        | SessionState::AwaitingVerification { .. }
        | SessionState::Verifying { .. }
        | SessionState::AwaitingBootstrapConfirmation { .. }
        | SessionState::Rejecting { .. }
        | SessionState::LoggingOut
        | SessionState::Locked(_)
        | SessionState::CapabilityBlocked { .. }
        | SessionState::SwitchingAccount { .. } => None,
    }
}

#[cfg(test)]
mod test_support;
