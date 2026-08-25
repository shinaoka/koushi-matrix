//! TimelineActor: per-`TimelineKey` subscription, diff relay, pagination,
//! send/edit/redact.
//!
//! ## Ownership
//! One `TimelineActor` per `TimelineKey`, owned by `AccountActor` in a
//! `HashMap<TimelineKey, TimelineActorHandle>`. `Unsubscribe` removes and drops
//! the entry — the runtime never leaks timeline state (Async rule 7).
//!
//! ## Generations & overflow protocol (canon pre-resolved decision B)
//! The relay task holds an `mpsc::Sender<DiffBatch>` of capacity 128
//! (`TIMELINE_DIFF_QUEUE_CAPACITY`). On `try_send` overflow:
//!   1. Stop forwarding diffs for the current generation.
//!   2. Bump `TimelineGeneration` (stored in `Arc<AtomicU64>`).
//!   3. Emit `ResyncRequired { reason: QueueOverflow }`.
//!   4. Emit a fresh `InitialItems` with the new generation from the current
//!      SDK timeline snapshot.
//!
//! ## Batch IDs (canon pre-resolved decision C)
//! Monotonic per generation starting at 0; the relay task increments
//! `next_batch_id` before emitting each `ItemsUpdated`.
//!
//! ## Transaction ID mapping (canon pre-resolved decision D)
//! The stable timeline manager registers each client transaction/request before
//! enqueue. Its supervised worker binds that registration to the SDK transaction
//! returned by `Timeline::send`/the attachment SendHandle. The sole session-global
//! send-queue observer then correlates exact SDK terminals and emits manager-owned
//! reducer/completion handoffs. Replaceable actor-local room observers use SDK
//! transaction IDs only for presentation state.
//!
//! ## Pagination
//! `Timeline::paginate_backwards(n)` returns `Ok(true)` when the start of
//! history is reached (EndReached), `Ok(false)` when more history exists, and
//! `Err(_)` on failure. We emit:
//!   Idle → Paginating → (EndReached | Idle | Failed)
//! Forward pagination is only allowed on Focused timelines (Async rule 5).
//!
//! ## Thread/Focused support
//! The vendored SDK supports `TimelineFocus::Thread` and `TimelineFocus::Event`
//! (`::Focused`). Both are implemented. paginate_forwards is valid on Focused
//! (SDK: returns Ok(true) for Live focus, actually does work for Event focus).
//!
//! ## SDK handle lifecycle
//! The `Arc<matrix_sdk_ui::Timeline>` is held by the relay task. Dropping the
//! relay task's sender (on Unsubscribe or AccountActor shutdown) cancels the
//! relay task, which drops the Timeline handle — cancelling its background tasks.
//!
//! ## Security
//! Message bodies appear in `TimelineItem.body` (visible UI state per canon)

mod actor;
mod composer;
mod diagnostics;
mod display_projection;
mod gap_repair;
mod item_projection;
mod manager;
mod media;
mod navigation;
mod outbound_send;
mod read_state;
mod relay;
mod residency;
mod room_key_recovery;
#[cfg(test)]
mod test_source;
#[cfg(test)]
mod test_support;
mod thread_projection;

pub(crate) use diagnostics::record_thread_summary_reconciliation;

#[cfg(test)]
#[allow(unused_imports)] // Preserve the baseline cfg-test flat path.
pub(crate) use composer::build_room_message_content_from_composer_document;
#[allow(unused_imports)] // Preserve the baseline crate-internal flat paths.
pub(crate) use composer::{
    build_room_message_content_from_composer_body,
    build_room_message_content_from_composer_body_with_options,
    validate_composer_body_for_timeline_send,
};
pub use item_projection::sdk_item_to_timeline_item;
#[allow(unused_imports)] // Preserve the baseline crate-internal flat paths.
pub(crate) use item_projection::{
    reaction_groups_from_sdk, timeline_item_can_edit, timeline_item_can_react,
    timeline_item_can_redact, validate_cancel_send, validate_redact_reaction, validate_retry_send,
    validate_send_reaction,
};
pub(crate) use manager::TimelineMessage;
pub use manager::{TIMELINE_DIFF_QUEUE_CAPACITY, TimelineManagerActor, TimelineManagerHandle};
pub use navigation::TimelineProjectionAcknowledgement;
#[cfg(any(test, feature = "qa-bin"))]
pub use navigation::display_projection_reset_fallback_count;
pub(crate) use navigation::{
    NavigationProjectionCleanup, NavigationProjectionIngress, NavigationProjectionIntent,
};
pub(crate) use read_state::{ReadPersistenceIngress, ReadPersistenceRequest};
pub(crate) use residency::{
    RoomMembershipTransition, RoomMembershipTransitionKind, RoomRemovalCause,
    TimelineSubscriptionResidencyHandle, TimelineSubscriptionResidencyPermit,
    VisibleRoomObservation,
};
