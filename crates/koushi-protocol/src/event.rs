//! Public transport-neutral event DTOs.

use serde::{Deserialize, Serialize};

use crate::{failure::CoreFailure, ids::RequestId, state_update::StateDelta};

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
pub use room::RoomEvent;
pub use search::{SearchEvent, SearchResultItem};
pub use timeline::{
    CjkTextPolicyEvent, LinkPreview, LinkPreviewImage, LinkPreviewState, PaginationDirection,
    PaginationState, ReactionGroup, ReactionSender, RoomKeyRequestStage, RoomKeyRequestStateDto,
    RoomKeyRequestWithheldCode, ThreadSummaryDto, ThreadsListEvent, TimelineAnchorRestoreStatus,
    TimelineBottomArrival, TimelineCodeBlock, TimelineDiff, TimelineDisplayKind,
    TimelineDisplayLabelUpdate, TimelineDisplayMetadata, TimelineEvent, TimelineFormattedBody,
    TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId, TimelineLinkRange,
    TimelineMedia, TimelineMediaKind, TimelineMediaSource, TimelineMediaThumbnail,
    TimelineMegolmSessionReason, TimelineMessageActions, TimelineMessageKind,
    TimelineMessageSource, TimelineNavigationSnapshot, TimelineNoticeI18n, TimelineNoticeI18nKey,
    TimelineReadStateSync, TimelineResyncReason, TimelineSendFailureReason, TimelineSendState,
    TimelineSpoilerSpan, TimelineUnableToDecrypt, TimelineUnableToDecryptReason,
    TimelineUnreadPosition, TimelineViewportObservation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    Event,
    Room,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentNoOpReason {
    SessionNotReady,
    RoomNotInState,
    AlreadyActive,
    TimelineTargetMissing,
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum IntentOutcome {
    Committed,
    BenignNoOp(IntentNoOpReason),
    FailedNoOp(IntentNoOpReason),
}

#[derive(Clone, Debug)]
pub enum CoreEvent {
    StateDelta(StateDelta),
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
    IntentLifecycle {
        request_id: RequestId,
        outcome: IntentOutcome,
        published_generation: u64,
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

#[cfg(test)]
mod test_support;
