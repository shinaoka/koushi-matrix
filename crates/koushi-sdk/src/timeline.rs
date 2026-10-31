use crate::room_projection::{
    matrix_room, matrix_timeline_update_from_ui, matrix_timeline_updates_from_diffs, timeline_room,
};
use crate::{MatrixClientSession, MatrixRoomOperationError, MatrixRoomOperationFailureKind};
use futures_util::{Stream, StreamExt};
use std::{fmt, pin::Pin, sync::Arc, time::Duration};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixTimelineContinuity {
    Unknown,
    Gapped,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixTimelineGapError {
    InvalidRoom,
    RoomUnavailable,
    Sdk,
}

#[derive(Clone)]
pub struct MatrixTimelineGapHandle {
    room_id: matrix_sdk::ruma::OwnedRoomId,
    descriptor: matrix_sdk::event_cache::RoomTimelineGapDescriptor,
}

/// Token-free room-subscription response provenance used by Core.
#[derive(Clone)]
pub struct MatrixRoomSubscriptionCheckpoint {
    subscription_generation: u64,
    room_id: matrix_sdk::ruma::OwnedRoomId,
    limited: bool,
    event_count: usize,
    prev_batch_present: bool,
    has_timeline_update: bool,
    inserted_gap: Option<matrix_sdk::event_cache::RoomTimelineGapDescriptor>,
}

/// Backend-neutral, token-free room timeline provenance committed by the SDK.
#[derive(Clone)]
pub struct MatrixCommittedRoomTimelineCheckpoint {
    generation: u64,
    response_sequence: u64,
    observation_sequence: Option<u64>,
    room_id: matrix_sdk::ruma::OwnedRoomId,
    has_timeline_update: bool,
    inserted_gap: Option<matrix_sdk::event_cache::RoomTimelineGapDescriptor>,
}

impl MatrixCommittedRoomTimelineCheckpoint {
    pub fn from_room_subscription(
        checkpoint: &matrix_sdk_ui::room_list_service::RoomSubscriptionCheckpoint,
    ) -> Self {
        let timeline = checkpoint.timeline();
        Self {
            generation: checkpoint.subscription_generation().get(),
            response_sequence: checkpoint.response_sequence(),
            observation_sequence: timeline.map(|observation| observation.sequence()),
            room_id: checkpoint.room_id().to_owned(),
            has_timeline_update: timeline.is_some(),
            inserted_gap: timeline.and_then(|observation| observation.inserted_gap().cloned()),
        }
    }

    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn from_gap_for_testing(
        generation: u64,
        response_sequence: u64,
        observation_sequence: u64,
        gap: &MatrixTimelineGapHandle,
    ) -> Self {
        Self {
            generation,
            response_sequence,
            observation_sequence: Some(observation_sequence),
            room_id: gap.room_id.clone(),
            has_timeline_update: true,
            inserted_gap: Some(gap.descriptor.clone()),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn response_sequence(&self) -> u64 {
        self.response_sequence
    }

    pub fn observation_sequence(&self) -> Option<u64> {
        self.observation_sequence
    }

    pub fn room_id(&self) -> &str {
        self.room_id.as_str()
    }

    pub fn has_timeline_update(&self) -> bool {
        self.has_timeline_update
    }

    pub fn has_inserted_gap(&self) -> bool {
        self.inserted_gap.is_some()
    }

    pub fn inserted_gap_handle(&self) -> Option<MatrixTimelineGapHandle> {
        self.inserted_gap
            .clone()
            .map(|descriptor| MatrixTimelineGapHandle {
                room_id: self.room_id.clone(),
                descriptor,
            })
    }

    pub fn matches_gap(&self, gap: &MatrixTimelineGapHandle) -> bool {
        self.room_id == gap.room_id
            && self
                .inserted_gap
                .as_ref()
                .is_some_and(|descriptor| descriptor == &gap.descriptor)
    }

    pub fn same_response_as(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.room_id == other.room_id
            && self.response_sequence == other.response_sequence
    }
}

impl std::fmt::Debug for MatrixCommittedRoomTimelineCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixCommittedRoomTimelineCheckpoint")
            .field("generation", &self.generation)
            .field("response_sequence", &self.response_sequence)
            .field("observation_sequence", &self.observation_sequence)
            .field("has_timeline_update", &self.has_timeline_update)
            .field("has_inserted_gap", &self.inserted_gap.is_some())
            .finish()
    }
}

#[cfg(test)]
mod committed_room_timeline_checkpoint_tests {
    use super::MatrixCommittedRoomTimelineCheckpoint;

    #[test]
    fn checkpoint_identity_is_engine_neutral_and_debug_is_private_safe() {
        let room_id = matrix_sdk::ruma::room_id!("!private-room:example.org");
        let checkpoint = MatrixCommittedRoomTimelineCheckpoint {
            generation: 41,
            response_sequence: 11,
            observation_sequence: Some(7),
            room_id: room_id.to_owned(),
            has_timeline_update: true,
            inserted_gap: None,
        };
        let same_response = checkpoint.clone();
        let different_observation = MatrixCommittedRoomTimelineCheckpoint {
            observation_sequence: Some(8),
            ..checkpoint.clone()
        };
        let different_response = MatrixCommittedRoomTimelineCheckpoint {
            response_sequence: 12,
            ..checkpoint.clone()
        };
        let different_generation = MatrixCommittedRoomTimelineCheckpoint {
            generation: 42,
            ..checkpoint.clone()
        };
        let different_room = MatrixCommittedRoomTimelineCheckpoint {
            room_id: matrix_sdk::ruma::room_id!("!other-room:example.org").to_owned(),
            ..checkpoint.clone()
        };

        assert_eq!(checkpoint.generation(), 41);
        assert_eq!(checkpoint.response_sequence(), 11);
        assert_eq!(checkpoint.observation_sequence(), Some(7));
        assert_eq!(checkpoint.room_id(), room_id.as_str());
        assert!(checkpoint.has_timeline_update());
        assert!(!checkpoint.has_inserted_gap());
        assert!(checkpoint.same_response_as(&same_response));
        assert!(checkpoint.same_response_as(&different_observation));
        assert!(!checkpoint.same_response_as(&different_response));
        assert!(!checkpoint.same_response_as(&different_generation));
        assert!(!checkpoint.same_response_as(&different_room));
        let debug = format!("{checkpoint:?}");
        assert!(debug.contains("generation: 41"));
        assert!(debug.contains("response_sequence: 11"));
        assert!(debug.contains("observation_sequence: Some(7)"));
        assert!(debug.contains("has_timeline_update: true"));
        assert!(debug.contains("has_inserted_gap: false"));
        assert!(!debug.contains("backend"));
        assert!(!debug.contains("origin"));
        assert!(!debug.contains(room_id.as_str()));
        assert!(!debug.contains("private-token"));
    }
}

impl MatrixRoomSubscriptionCheckpoint {
    /// Convert the SDK UI checkpoint without exposing its private gap token.
    pub fn from_sdk(
        checkpoint: &matrix_sdk_ui::room_list_service::RoomSubscriptionCheckpoint,
    ) -> Self {
        let timeline = checkpoint.timeline();
        Self {
            subscription_generation: checkpoint.subscription_generation().get(),
            room_id: checkpoint.room_id().to_owned(),
            limited: timeline.is_some_and(|observation| observation.limited()),
            event_count: timeline.map_or(0, |observation| observation.event_count()),
            prev_batch_present: timeline
                .is_some_and(|observation| observation.prev_batch_present()),
            has_timeline_update: timeline.is_some(),
            inserted_gap: timeline.and_then(|observation| observation.inserted_gap().cloned()),
        }
    }

    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn from_gap_for_testing(
        subscription_generation: u64,
        gap: &MatrixTimelineGapHandle,
    ) -> Self {
        Self {
            subscription_generation,
            room_id: gap.room_id.clone(),
            limited: true,
            event_count: 1,
            prev_batch_present: true,
            has_timeline_update: true,
            inserted_gap: Some(gap.descriptor.clone()),
        }
    }

    pub fn subscription_generation(&self) -> u64 {
        self.subscription_generation
    }

    pub fn room_id(&self) -> &str {
        self.room_id.as_str()
    }

    pub fn has_timeline_update(&self) -> bool {
        self.has_timeline_update
    }

    pub fn limited(&self) -> bool {
        self.limited
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    pub fn prev_batch_present(&self) -> bool {
        self.prev_batch_present
    }

    pub fn has_inserted_gap(&self) -> bool {
        self.inserted_gap.is_some()
    }

    pub fn matches_gap(&self, gap: &MatrixTimelineGapHandle) -> bool {
        self.room_id == gap.room_id
            && self
                .inserted_gap
                .as_ref()
                .is_some_and(|descriptor| descriptor == &gap.descriptor)
    }
}

impl std::fmt::Debug for MatrixRoomSubscriptionCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixRoomSubscriptionCheckpoint")
            .field("subscription_generation", &self.subscription_generation)
            .field("limited", &self.limited)
            .field("event_count", &self.event_count)
            .field("prev_batch_present", &self.prev_batch_present)
            .field("has_timeline_update", &self.has_timeline_update)
            .field("has_inserted_gap", &self.inserted_gap.is_some())
            .finish()
    }
}

impl std::fmt::Debug for MatrixTimelineGapHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixTimelineGapHandle")
            .finish_non_exhaustive()
    }
}

impl MatrixTimelineGapHandle {
    /// Coarse persisted-topology revision used by Core to detect an unchanged
    /// gap selection. The opaque descriptor, token, and boundary identities
    /// remain SDK-owned and actor-private.
    pub fn topology_revision(&self) -> u64 {
        self.descriptor.revision
    }

    pub fn older_boundary_event_id(&self) -> Option<&str> {
        self.descriptor
            .older_event_id
            .as_deref()
            .map(|event_id| event_id.as_str())
    }

    pub fn newer_boundary_event_id(&self) -> Option<&str> {
        self.descriptor
            .newer_event_id
            .as_deref()
            .map(|event_id| event_id.as_str())
    }
}

/// Content-free counters for one room-timeline gap inspection.
///
/// Aggregate counts only: never contains room or event identifiers,
/// pagination tokens, message content, or local paths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixTimelineGapInspectionDiagnostics {
    /// Number of persisted chunks whose metadata was scanned.
    pub scanned_chunks: usize,
    /// Number of event bodies decoded while resolving gap boundaries.
    pub event_bodies_decoded: usize,
    /// Wall-clock duration of the inspection, in milliseconds.
    pub duration_ms: u64,
}

#[derive(Clone)]
pub struct MatrixTimelineGapInspection {
    pub continuity: MatrixTimelineContinuity,
    pub gaps: Vec<MatrixTimelineGapHandle>,
    /// Content-free counters for this inspection.
    pub diagnostics: MatrixTimelineGapInspectionDiagnostics,
}

impl std::fmt::Debug for MatrixTimelineGapInspection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixTimelineGapInspection")
            .field("continuity", &self.continuity)
            .field("gap_count", &self.gaps.len())
            .field("scanned_chunks", &self.diagnostics.scanned_chunks)
            .field(
                "event_bodies_decoded",
                &self.diagnostics.event_bodies_decoded,
            )
            .field("duration_ms", &self.diagnostics.duration_ms)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixTimelineGapRepairBudget {
    pub event_limit: u16,
    pub cached_chunk_limit: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixTimelineGapRepairOutcome {
    Stale,
    Deferred { cached_chunks_loaded: usize },
    Failed,
    Progress { events: usize },
    BoundariesJoined { events: usize },
    StartReached { events: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixTimelineGapRepairResult {
    pub outcome: MatrixTimelineGapRepairOutcome,
    pub last_projection_batch: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixLiveTailRefreshOutcome {
    Cancelled,
    Unchanged,
    Advanced {
        events: usize,
    },
    Detached {
        events: usize,
        historical_gap_remaining: bool,
    },
    Stale,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatrixLiveTailRefreshDiagnostics {
    pub cached_suffix_events: usize,
    pub response_events_with_ids: usize,
    pub newest_cached_response_index: Option<usize>,
    pub older_anchor_response_index: Option<usize>,
    pub in_memory_duplicates: usize,
    pub in_store_duplicates: usize,
    pub new_events: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixLiveTailRefreshResult {
    pub outcome: MatrixLiveTailRefreshOutcome,
    pub returned_events: usize,
    pub diagnostics: MatrixLiveTailRefreshDiagnostics,
    pub last_projection_batch: Option<u32>,
}

#[derive(Clone, Default)]
pub struct MatrixLiveTailRefreshCancellation {
    inner: matrix_sdk::event_cache::RoomLiveTailRefreshCancellation,
}

impl MatrixLiveTailRefreshCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }
}

impl std::fmt::Debug for MatrixLiveTailRefreshCancellation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MatrixLiveTailRefreshCancellation(..)")
    }
}

fn failed_live_tail_refresh_result() -> MatrixLiveTailRefreshResult {
    MatrixLiveTailRefreshResult {
        outcome: MatrixLiveTailRefreshOutcome::Failed,
        returned_events: 0,
        diagnostics: MatrixLiveTailRefreshDiagnostics::default(),
        last_projection_batch: None,
    }
}

fn map_live_tail_refresh_result(
    result: matrix_sdk::event_cache::Result<matrix_sdk::event_cache::RoomLiveTailRefreshResult>,
) -> MatrixLiveTailRefreshResult {
    use matrix_sdk::event_cache::RoomLiveTailRefreshOutcome;

    let Ok(result) = result else {
        return failed_live_tail_refresh_result();
    };
    let outcome = match result.outcome {
        RoomLiveTailRefreshOutcome::Cancelled => MatrixLiveTailRefreshOutcome::Cancelled,
        RoomLiveTailRefreshOutcome::Unchanged => MatrixLiveTailRefreshOutcome::Unchanged,
        RoomLiveTailRefreshOutcome::Advanced { events } => {
            MatrixLiveTailRefreshOutcome::Advanced { events }
        }
        RoomLiveTailRefreshOutcome::Detached {
            events,
            historical_gap_remaining,
        } => MatrixLiveTailRefreshOutcome::Detached {
            events,
            historical_gap_remaining,
        },
        RoomLiveTailRefreshOutcome::Stale => MatrixLiveTailRefreshOutcome::Stale,
        RoomLiveTailRefreshOutcome::Failed => MatrixLiveTailRefreshOutcome::Failed,
    };
    MatrixLiveTailRefreshResult {
        outcome,
        returned_events: result.returned_events,
        diagnostics: MatrixLiveTailRefreshDiagnostics {
            cached_suffix_events: result.diagnostics.cached_suffix_events,
            response_events_with_ids: result.diagnostics.response_events_with_ids,
            newest_cached_response_index: result.diagnostics.newest_cached_response_index,
            older_anchor_response_index: result.diagnostics.older_anchor_response_index,
            in_memory_duplicates: result.diagnostics.in_memory_duplicates,
            in_store_duplicates: result.diagnostics.in_store_duplicates,
            new_events: result.diagnostics.new_events,
        },
        last_projection_batch: result.last_projection_batch,
    }
}

#[cfg(test)]
mod matrix_live_tail_refresh_mapping_tests {
    use super::{
        MatrixLiveTailRefreshDiagnostics, MatrixLiveTailRefreshOutcome,
        MatrixLiveTailRefreshResult, map_live_tail_refresh_result,
    };
    use matrix_sdk::event_cache::{
        EventCacheError, RoomLiveTailRefreshDiagnostics as SdkDiagnostics,
        RoomLiveTailRefreshOutcome as SdkOutcome, RoomLiveTailRefreshResult as SdkResult,
    };

    #[test]
    fn sdk_live_tail_outcomes_map_one_for_one_with_projection_metadata() {
        let cases = [
            (
                SdkOutcome::Cancelled,
                MatrixLiveTailRefreshOutcome::Cancelled,
            ),
            (
                SdkOutcome::Unchanged,
                MatrixLiveTailRefreshOutcome::Unchanged,
            ),
            (
                SdkOutcome::Advanced { events: 3 },
                MatrixLiveTailRefreshOutcome::Advanced { events: 3 },
            ),
            (
                SdkOutcome::Detached {
                    events: 5,
                    historical_gap_remaining: true,
                },
                MatrixLiveTailRefreshOutcome::Detached {
                    events: 5,
                    historical_gap_remaining: true,
                },
            ),
            (SdkOutcome::Stale, MatrixLiveTailRefreshOutcome::Stale),
            (SdkOutcome::Failed, MatrixLiveTailRefreshOutcome::Failed),
        ];

        for (sdk_outcome, expected_outcome) in cases {
            let mapped = map_live_tail_refresh_result(Ok(SdkResult {
                outcome: sdk_outcome,
                returned_events: 9,
                diagnostics: SdkDiagnostics {
                    cached_suffix_events: 7,
                    response_events_with_ids: 9,
                    newest_cached_response_index: Some(0),
                    older_anchor_response_index: Some(4),
                    in_memory_duplicates: 3,
                    in_store_duplicates: 2,
                    new_events: 4,
                },
                last_projection_batch: Some(1),
            }));

            assert_eq!(
                mapped,
                MatrixLiveTailRefreshResult {
                    outcome: expected_outcome,
                    returned_events: 9,
                    diagnostics: MatrixLiveTailRefreshDiagnostics {
                        cached_suffix_events: 7,
                        response_events_with_ids: 9,
                        newest_cached_response_index: Some(0),
                        older_anchor_response_index: Some(4),
                        in_memory_duplicates: 3,
                        in_store_duplicates: 2,
                        new_events: 4,
                    },
                    last_projection_batch: Some(1),
                }
            );
        }
    }

    #[test]
    fn sdk_live_tail_errors_map_to_private_data_free_failure() {
        let mapped =
            map_live_tail_refresh_result(Err(EventCacheError::InvalidLinkedChunkMetadata {
                details: "raw token for !private-room:example.invalid".to_owned(),
            }));

        assert_eq!(
            mapped,
            MatrixLiveTailRefreshResult {
                outcome: MatrixLiveTailRefreshOutcome::Failed,
                returned_events: 0,
                diagnostics: MatrixLiveTailRefreshDiagnostics::default(),
                last_projection_batch: None,
            }
        );
        let debug = format!("{mapped:?}");
        assert!(!debug.contains("raw token"));
        assert!(!debug.contains("!private-room"));
    }
}

impl MatrixClientSession {
    pub async fn inspect_room_timeline_gaps(
        &self,
        room_id: &str,
    ) -> Result<MatrixTimelineGapInspection, MatrixTimelineGapError> {
        use matrix_sdk::event_cache::RoomTimelineContinuity;

        let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
            .map_err(|_| MatrixTimelineGapError::InvalidRoom)?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or(MatrixTimelineGapError::RoomUnavailable)?;
        let (cache, _drop_handles) = self
            .client
            .event_cache()
            .ok_or(MatrixTimelineGapError::Sdk)?
            .room(room.room_id())
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let inspection = cache
            .inspect_timeline_gaps()
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let continuity = match inspection.continuity {
            RoomTimelineContinuity::Unknown => MatrixTimelineContinuity::Unknown,
            RoomTimelineContinuity::Gapped => MatrixTimelineContinuity::Gapped,
            RoomTimelineContinuity::Complete => MatrixTimelineContinuity::Complete,
        };
        let gaps = inspection
            .gaps
            .into_iter()
            .map(|descriptor| MatrixTimelineGapHandle {
                room_id: room_id.clone(),
                descriptor,
            })
            .collect();
        Ok(MatrixTimelineGapInspection {
            continuity,
            gaps,
            diagnostics: MatrixTimelineGapInspectionDiagnostics {
                scanned_chunks: inspection.diagnostics.scanned_chunks,
                event_bodies_decoded: inspection.diagnostics.event_bodies_decoded,
                duration_ms: inspection.diagnostics.duration_ms,
            },
        })
    }
    pub async fn repair_room_timeline_gap(
        &self,
        gap: &MatrixTimelineGapHandle,
        budget: MatrixTimelineGapRepairBudget,
        actor_generation: u64,
        repair_generation: u64,
    ) -> Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError> {
        use matrix_sdk::event_cache::{
            RoomTimelineGapProjectionId, RoomTimelineGapRepairBudget, RoomTimelineGapRepairOutcome,
        };

        let room = self
            .client
            .get_room(&gap.room_id)
            .ok_or(MatrixTimelineGapError::RoomUnavailable)?;
        let (cache, _drop_handles) = self
            .client
            .event_cache()
            .ok_or(MatrixTimelineGapError::Sdk)?
            .room(room.room_id())
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let result = cache
            .pagination()
            .repair_timeline_gap_with_projection(
                &gap.descriptor,
                RoomTimelineGapRepairBudget {
                    event_limit: budget.event_limit,
                    cached_chunk_limit: budget.cached_chunk_limit,
                },
                RoomTimelineGapProjectionId {
                    actor_generation,
                    repair_generation,
                },
            )
            .await
            .map_err(|_| MatrixTimelineGapError::Sdk)?;
        let outcome = match result.outcome {
            RoomTimelineGapRepairOutcome::Stale => MatrixTimelineGapRepairOutcome::Stale,
            RoomTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded,
            } => MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded,
            },
            RoomTimelineGapRepairOutcome::Failed => MatrixTimelineGapRepairOutcome::Failed,
            RoomTimelineGapRepairOutcome::Progress { events } => {
                MatrixTimelineGapRepairOutcome::Progress { events }
            }
            RoomTimelineGapRepairOutcome::BoundariesJoined { events } => {
                MatrixTimelineGapRepairOutcome::BoundariesJoined { events }
            }
            RoomTimelineGapRepairOutcome::StartReached { events } => {
                MatrixTimelineGapRepairOutcome::StartReached { events }
            }
        };
        Ok(MatrixTimelineGapRepairResult {
            outcome,
            last_projection_batch: result.last_projection_batch,
        })
    }
    pub async fn refresh_room_live_tail(
        &self,
        room_id: &str,
        event_limit: u16,
        actor_generation: u64,
        operation_generation: u64,
        cancellation: MatrixLiveTailRefreshCancellation,
    ) -> MatrixLiveTailRefreshResult {
        let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(room_id) else {
            return failed_live_tail_refresh_result();
        };
        let Some(room) = self.client.get_room(&room_id) else {
            return failed_live_tail_refresh_result();
        };
        let Some(event_cache) = self.client.event_cache() else {
            return failed_live_tail_refresh_result();
        };
        let Ok((cache, _drop_handles)) = event_cache.room(room.room_id()).await else {
            return failed_live_tail_refresh_result();
        };
        map_live_tail_refresh_result(
            cache
                .pagination()
                .refresh_live_tail_with_projection(
                    event_limit,
                    matrix_sdk::event_cache::RoomTimelineGapProjectionId {
                        actor_generation,
                        repair_generation: operation_generation,
                    },
                    cancellation.inner,
                )
                .await,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixTimelineItem {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: String,
    /// Thread root when this item is a thread reply, so unread-resolved
    /// Activity rows route to the Thread panel (#965).
    pub thread_root: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixTimelineUpdate {
    Upsert(MatrixTimelineItem),
    Remove { room_id: String, event_id: String },
}

pub type MatrixTimelineUpdateStream = Pin<Box<dyn Stream<Item = Vec<MatrixTimelineUpdate>> + Send>>;

pub struct MatrixTimelineSubscription {
    room_id: String,
    timeline: Arc<matrix_sdk_ui::Timeline>,
    initial_items: Vec<MatrixTimelineItem>,
    updates: MatrixTimelineUpdateStream,
}

#[derive(Clone)]
pub struct MatrixTimelinePaginationHandle {
    timeline: Arc<matrix_sdk_ui::Timeline>,
}

impl MatrixTimelinePaginationHandle {
    pub async fn paginate_backwards(&self, event_count: u16) -> Result<bool, MatrixTimelineError> {
        self.timeline
            .paginate_backwards(event_count)
            .await
            .map_err(|_| MatrixTimelineError::Sdk)
    }
}

impl fmt::Debug for MatrixTimelinePaginationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixTimelinePaginationHandle")
            .field("timeline", &"Timeline(..)")
            .finish()
    }
}

impl MatrixTimelineSubscription {
    pub fn initial_items(&self) -> &[MatrixTimelineItem] {
        &self.initial_items
    }

    pub async fn next_update(&mut self) -> Option<Vec<MatrixTimelineUpdate>> {
        self.updates.next().await
    }

    pub fn pagination_handle(&self) -> MatrixTimelinePaginationHandle {
        MatrixTimelinePaginationHandle {
            timeline: self.timeline.clone(),
        }
    }

    pub async fn current_items(&mut self) -> Vec<MatrixTimelineItem> {
        self.timeline
            .items()
            .await
            .iter()
            .filter_map(|item| matrix_timeline_item_from_ui(&self.room_id, item))
            .collect()
    }

    pub async fn paginate_backwards(&self, event_count: u16) -> Result<bool, MatrixTimelineError> {
        self.timeline
            .paginate_backwards(event_count)
            .await
            .map_err(|_| MatrixTimelineError::Sdk)
    }
}

impl fmt::Debug for MatrixTimelineSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixTimelineSubscription")
            .field("initial_item_count", &self.initial_items.len())
            .field("timeline", &"Timeline(..)")
            .field("updates", &"TimelineUpdateStream(..)")
            .finish()
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixTimelineError {
    #[error("Matrix room id is invalid")]
    InvalidRoomId,
    #[error("Matrix room is not available")]
    RoomUnavailable,
    #[error("Matrix timeline operation failed")]
    Sdk,
}

pub fn subscribe_room_timeline_blocking(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixTimelineSubscription, MatrixTimelineError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixTimelineError::Sdk)?;

    runtime.block_on(subscribe_room_timeline(session, room_id))
}

pub fn room_timeline_visible_items_blocking(
    session: &MatrixClientSession,
    room_id: &str,
    backfill_event_count: u16,
) -> Result<Vec<MatrixTimelineItem>, MatrixTimelineError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixTimelineError::Sdk)?;

    runtime.block_on(room_timeline_visible_items(
        session,
        room_id,
        backfill_event_count,
    ))
}

pub async fn send_text_message(
    session: &MatrixClientSession,
    room_id: &str,
    body: &str,
    transaction_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let txn_id = matrix_sdk::ruma::OwnedTransactionId::from(transaction_id);
    let content =
        matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(body);

    let result = room
        .send(content)
        .require_backed_up_session()
        .with_transaction_id(txn_id)
        .await
        .map(|_| ());
    map_room_send_result(result)
}

fn map_room_send_result(
    result: Result<(), matrix_sdk::Error>,
) -> Result<(), MatrixRoomOperationError> {
    result.map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Replace a text event's body.
///
/// Text-only by construction: the replacement carries no media payload, so
/// pointing this at an `m.image`/`m.file`/`m.audio`/`m.video` event would drop
/// its attachment (issue #328). The product edit path is
/// `TimelineCommand::EditText`, which resolves the target's message type and
/// edits a media caption in place instead.
pub async fn edit_text_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
    body: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;
    let content =
        matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation::text_plain(
            body,
        );
    let edit_content = matrix_sdk::room::edit::EditedContent::RoomMessage(content);
    let edit_event = room
        .make_edit_event(&event_id, edit_content)
        .await
        .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))?;

    room.send(edit_event)
        .require_backed_up_session()
        .await
        .map(|_| ())
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn redact_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let event_id = matrix_sdk::ruma::EventId::parse(event_id)
        .map_err(|_| MatrixRoomOperationError::InvalidEventId)?;

    room.redact(&event_id, None, None)
        .await
        .map(|_| ())
        .map_err(|_| MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Sdk))
}

pub async fn subscribe_room_timeline(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixTimelineSubscription, MatrixTimelineError> {
    let room = timeline_room(session, room_id)?;
    let timeline = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
        .build()
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;
    let timeline = Arc::new(timeline);
    let (items, updates) = timeline.subscribe().await;
    let initial_items = items
        .iter()
        .filter_map(|item| matrix_timeline_item_from_ui(room_id, item))
        .collect();
    let update_room_id = room_id.to_owned();
    let updates = updates
        .map(move |diffs| matrix_timeline_updates_from_diffs(&update_room_id, diffs))
        .boxed();

    Ok(MatrixTimelineSubscription {
        room_id: room_id.to_owned(),
        timeline,
        initial_items,
        updates,
    })
}

pub async fn room_timeline_visible_items(
    session: &MatrixClientSession,
    room_id: &str,
    backfill_event_count: u16,
) -> Result<Vec<MatrixTimelineItem>, MatrixTimelineError> {
    let room = timeline_room(session, room_id)?;
    let timeline = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
        .build()
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;
    let (items, updates) = timeline.subscribe().await;
    let mut items = items
        .iter()
        .filter_map(|item| matrix_timeline_item_from_ui(room_id, item))
        .collect::<Vec<_>>();
    if !items.is_empty() || backfill_event_count == 0 {
        return Ok(items);
    }

    let mut updates = Box::pin(updates);
    timeline
        .paginate_backwards(backfill_event_count)
        .await
        .map_err(|_| MatrixTimelineError::Sdk)?;

    for _ in 0..3 {
        let Some(diffs) = tokio::time::timeout(Duration::from_secs(10), updates.next())
            .await
            .map_err(|_| MatrixTimelineError::Sdk)?
        else {
            break;
        };
        items.extend(
            matrix_timeline_updates_from_diffs(room_id, diffs)
                .into_iter()
                .filter_map(|update| match update {
                    MatrixTimelineUpdate::Upsert(item) => Some(item),
                    MatrixTimelineUpdate::Remove { .. } => None,
                }),
        );
        if !items.is_empty() {
            break;
        }
    }

    Ok(items)
}

fn matrix_timeline_item_from_ui(
    room_id: &str,
    item: &matrix_sdk_ui::timeline::TimelineItem,
) -> Option<MatrixTimelineItem> {
    match matrix_timeline_update_from_ui(room_id, item)? {
        MatrixTimelineUpdate::Upsert(item) => Some(item),
        MatrixTimelineUpdate::Remove { .. } => None,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn send_wrapper_propagates_recipient_collection_failure() {
        assert_eq!(
            super::map_room_send_result(Err(matrix_sdk::Error::NoOlmMachine)),
            Err(super::MatrixRoomOperationError::Sdk(
                super::MatrixRoomOperationFailureKind::Encryption
            ))
        );
    }
    #[test]
    fn send_wrapper_maps_secure_backup_required_to_a_typed_closed_failure() {
        assert_eq!(
            super::map_room_send_result(Err(matrix_sdk::Error::SecureBackupRequired)),
            Err(super::MatrixRoomOperationError::Sdk(
                super::MatrixRoomOperationFailureKind::SecureBackupRequired
            ))
        );
        assert_eq!(
            super::MatrixRoomOperationFailureKind::SecureBackupRequired.to_string(),
            "secure_backup_required"
        );
    }
}
