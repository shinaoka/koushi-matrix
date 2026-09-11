//! ThreadsListActor: scoped thread list subscription and pagination.
//!
//! Wraps one SDK `ThreadListService` per room in the requested scope and
//! projects `ThreadListItem`s into the app-owned `ThreadsListItem` DTO. All
//! state transitions are delivered as typed `AppAction`s (and mirrored as
//! `CoreEvent::ThreadsList` events) so the reducer owns the UI snapshot.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{StreamExt, future::join_all};
use koushi_state::{AppAction, OperationFailureKind, ThreadsListItem, ThreadsListScope};
use matrix_sdk::ruma::RoomId;
use matrix_sdk_ui::timeline::thread_list_service::{
    ThreadListItem as SdkThreadListItem, ThreadListServiceError, ThreadRelationAggregate,
};
use matrix_sdk_ui::timeline::{ThreadListPaginationState, ThreadListService, TimelineDetails};
use tokio::sync::{broadcast, mpsc};

use crate::executor;
use crate::timeline::record_thread_summary_reconciliation;
use koushi_protocol::event::{CoreEvent, ThreadsListEvent, TimelineItem, TimelineItemId};
use koushi_protocol::ids::RequestId;

pub(crate) const THREAD_SUMMARY_PROJECTION_MAX_ROOTS: usize = 120;

const THREADS_LIST_SHUTDOWN_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const THREADS_LIST_SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Exact reply activity that requires a root outside the Room timeline's
/// canonical window. It is intentionally independent of `ThreadsListState`:
/// the side-panel service can be closed or paginated without affecting this
/// bounded room-timeline projection path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadRootProjectionActivity {
    pub room_id: String,
    pub root_event_id: String,
    pub activity_event_id: String,
    pub activity_timestamp_ms: Option<u64>,
    /// Live reply metadata is authoritative over a potentially stale bundled
    /// root summary when rendering the moved root's thread preview.
    pub activity_sender: Option<String>,
    pub activity_sender_label: Option<String>,
    pub activity_body_preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AggregateRefreshCause {
    InitialHydration,
    SelectedActivity,
    CanonicalBatch,
    /// The root left the accepted missing-root window through removal,
    /// redaction, clear, or reset.
    Removal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AggregateRefresh {
    pub activity: ThreadRootProjectionActivity,
    pub activity_revision: u64,
    pub summary_revision: u64,
    pub cause: AggregateRefreshCause,
    /// The reply activity still belongs to the bounded Room window.
    pub root_active: bool,
    /// The canonical root item is present in the bounded Room window.
    pub canonical_root_active: bool,
    pub hydrate_root: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthoritativeThreadAggregate {
    pub reply_count: u32,
    pub latest_event_id: Option<String>,
    pub latest_sender: Option<String>,
    pub latest_sender_label: Option<String>,
    pub latest_body_preview: Option<String>,
    pub latest_timestamp_ms: Option<u64>,
}

impl Default for AuthoritativeThreadAggregate {
    fn default() -> Self {
        Self {
            reply_count: 0,
            latest_event_id: None,
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ThreadRootProjectionRefreshResult {
    Hydrated {
        item: TimelineItem,
        aggregate: AuthoritativeThreadAggregate,
    },
    Aggregate(AuthoritativeThreadAggregate),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ThreadRootProjectionCompletion {
    Updated(ThreadRootProjectionRecord),
    Cleared(ThreadRootProjectionActivity),
    Ignored,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ThreadRootProjectionDecision {
    /// Start exactly one `Room::load_or_fetch_event(root_id, None)` request.
    StartFetch(ThreadRootProjectionActivity),
    /// The existing request remains bounded to one fetch, but a newer reply
    /// changed the presentation activity for the same root.
    ActivityUpdated(ThreadRootProjectionRecord),
    /// A retained request/result belongs to the currently active canonical
    /// reply window. Re-emitting it lets a replacement Room actor restore its
    /// pending/ready/failed display state without another fetch.
    Existing(ThreadRootProjectionRecord),
    /// A serial exhausted. The root is retired and must never reuse a counter.
    Retired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ThreadRootProjectionAttempt {
    Pending,
    /// A canonical Room root needs the shared aggregate, but not a root fetch.
    Canonical,
    Ready,
    Failed(OperationFailureKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LiveActivityFloor {
    activity: ThreadRootProjectionActivity,
    reply_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ThreadRootProjectionRecord {
    pub activity: ThreadRootProjectionActivity,
    pub aggregate: AuthoritativeThreadAggregate,
    pub activity_revision: u64,
    pub summary_revision: u64,
    /// One complete renderable root snapshot, retained for canonical roots and
    /// hydrated off-window roots alike.
    root_item: Option<TimelineItem>,
    aggregate_refresh: Option<AggregateRefresh>,
    aggregate_failure: Option<OperationFailureKind>,
    live_activity_floor: Option<LiveActivityFloor>,
    pending_rollback: Option<AuthoritativeThreadAggregate>,
    invalidated_activity: Option<(String, Option<u64>)>,
    pub retired: bool,
    attempt: ThreadRootProjectionAttempt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ThreadRootDisplayData {
    pub root_event_id: String,
    pub activity_event_id: String,
    pub activity_timestamp_ms: Option<u64>,
    pub item: Option<TimelineItem>,
    pub aggregate: AuthoritativeThreadAggregate,
    pub pending: bool,
    pub failure_kind: Option<OperationFailureKind>,
}

impl ThreadRootProjectionRecord {
    pub(crate) fn display_data(&self) -> ThreadRootDisplayData {
        ThreadRootDisplayData {
            root_event_id: self.activity.root_event_id.clone(),
            activity_event_id: self.activity.activity_event_id.clone(),
            activity_timestamp_ms: self.activity.activity_timestamp_ms,
            item: self.root_item.clone(),
            aggregate: effective_aggregate(self),
            // Summary refreshes must not replace an accepted root body with
            // a hydration placeholder. Keep operation status on the record;
            // only roots without content need a loading/error display row.
            pending: self.root_item.is_none() && self.is_pending(),
            failure_kind: if self.root_item.is_none() {
                self.failure_kind()
            } else {
                None
            },
        }
    }

    pub(crate) fn item(&self) -> Option<&TimelineItem> {
        self.root_item.as_ref()
    }

    pub(crate) fn failure_kind(&self) -> Option<OperationFailureKind> {
        self.aggregate_failure.or_else(|| match self.attempt {
            ThreadRootProjectionAttempt::Failed(kind) => Some(kind),
            ThreadRootProjectionAttempt::Pending
            | ThreadRootProjectionAttempt::Canonical
            | ThreadRootProjectionAttempt::Ready => None,
        })
    }

    pub(crate) fn is_hydration_pending(&self) -> bool {
        matches!(self.attempt, ThreadRootProjectionAttempt::Pending)
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.is_hydration_pending() || self.aggregate_refresh.is_some()
    }

    pub(crate) fn pending_refresh(&self) -> Option<AggregateRefresh> {
        self.aggregate_refresh.clone()
    }
}

/// Per-Room-timeline dedupe and terminal-state service for old thread roots.
///
/// This service owns no `Timeline` and has no pagination capability. The
/// actor that owns it performs the one bounded event-cache/network request
/// after `StartFetch`, then reports `mark_ready` or `mark_failed` exactly
/// once. Retaining failed attempts prevents repeated live reply diffs from
/// creating a fetch loop.
#[derive(Default)]
pub(crate) struct ThreadRootProjectionService {
    attempts: HashMap<(String, String), ThreadRootProjectionRecord>,
    active_root_event_ids: HashMap<String, HashSet<String>>,
    canonical_root_event_ids: HashMap<String, HashSet<String>>,
    diagnostic_ordinals: ThreadSummaryDiagnosticOrdinals,
}

#[derive(Default)]
struct ThreadSummaryDiagnosticOrdinals {
    rooms: HashMap<String, u64>,
    roots: HashMap<(String, String), u64>,
    next_room: u64,
    next_root: u64,
}

impl ThreadSummaryDiagnosticOrdinals {
    fn ordinals(&mut self, activity: &ThreadRootProjectionActivity) -> (u64, u64) {
        if !self.rooms.contains_key(&activity.room_id) {
            let ordinal = self.next_room;
            self.next_room = self.next_room.saturating_add(1);
            self.rooms.insert(activity.room_id.clone(), ordinal);
        }
        let root_key = (activity.room_id.clone(), activity.root_event_id.clone());
        if !self.roots.contains_key(&root_key) {
            let ordinal = self.next_root;
            self.next_root = self.next_root.saturating_add(1);
            self.roots.insert(root_key.clone(), ordinal);
        }
        (self.rooms[&activity.room_id], self.roots[&root_key])
    }

    fn remove_root(&mut self, room_id: &str, root_event_id: &str) {
        self.roots
            .remove(&(room_id.to_owned(), root_event_id.to_owned()));
    }

    fn clear_room(&mut self, room_id: &str) {
        self.rooms.remove(room_id);
        self.roots
            .retain(|(entry_room_id, _), _| entry_room_id != room_id);
    }
}

impl ThreadRootProjectionService {
    pub(crate) fn seed_canonical_root(&mut self, room_id: &str, item: &TimelineItem) {
        let TimelineItemId::Event {
            event_id: root_event_id,
        } = &item.id
        else {
            return;
        };
        let Some(summary) = item.thread_summary.as_ref() else {
            return;
        };
        let Some(latest_event_id) = summary
            .latest_event_id
            .as_deref()
            .map(str::trim)
            .filter(|event_id| !event_id.is_empty())
        else {
            return;
        };
        let activity = ThreadRootProjectionActivity {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
            activity_event_id: latest_event_id.to_owned(),
            activity_timestamp_ms: summary.latest_timestamp_ms,
            activity_sender: summary.latest_sender.clone(),
            activity_sender_label: summary.latest_sender_label.clone(),
            activity_body_preview: summary.latest_body_preview.clone(),
        };
        let aggregate = AuthoritativeThreadAggregate {
            reply_count: summary.reply_count,
            latest_event_id: Some(latest_event_id.to_owned()),
            latest_sender: summary.latest_sender.clone(),
            latest_sender_label: summary.latest_sender_label.clone(),
            latest_body_preview: summary.latest_body_preview.clone(),
            latest_timestamp_ms: summary.latest_timestamp_ms,
        };
        self.canonical_root_event_ids
            .entry(room_id.to_owned())
            .or_default()
            .insert(root_event_id.to_owned());
        let key = (room_id.to_owned(), root_event_id.to_owned());
        if let Some(record) = self.attempts.get_mut(&key) {
            if record.retired {
                return;
            }
            record.root_item = Some(item.clone());
            if aggregate.reply_count < effective_aggregate(record).reply_count {
                // Bundled roots are provisional: an edit can expose its
                // replacement event identity and transient count. Only an
                // independently matching event-cache aggregate may roll the
                // accepted projection backward.
                record.pending_rollback = Some(aggregate);
            }
            if matches!(record.attempt, ThreadRootProjectionAttempt::Pending) {
                record.attempt = ThreadRootProjectionAttempt::Canonical;
            }
            return;
        }
        if self
            .attempts
            .keys()
            .filter(|(entry_room_id, _)| entry_room_id == room_id)
            .count()
            >= THREAD_SUMMARY_PROJECTION_MAX_ROOTS
        {
            return;
        }
        self.attempts.insert(
            key,
            ThreadRootProjectionRecord {
                activity,
                aggregate,
                activity_revision: 1,
                summary_revision: 0,
                root_item: Some(item.clone()),
                aggregate_refresh: None,
                aggregate_failure: None,
                live_activity_floor: None,
                pending_rollback: None,
                invalidated_activity: None,
                retired: false,
                attempt: ThreadRootProjectionAttempt::Canonical,
            },
        );
    }

    pub(crate) fn observe(
        &mut self,
        activity: ThreadRootProjectionActivity,
    ) -> ThreadRootProjectionDecision {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        if let Some(record) = self.attempts.get_mut(&key) {
            if record.retired {
                return ThreadRootProjectionDecision::Retired;
            }
            if activity != record.activity {
                let newer = activity_is_newer(&activity, &record.activity);
                let invalidated_latest = record
                    .invalidated_activity
                    .as_ref()
                    .is_some_and(|(event_id, _)| event_id == &record.activity.activity_event_id);
                if !newer && !invalidated_latest {
                    // A bounded Room/Thread window can temporarily omit the
                    // accepted latest reply. Only an explicit invalidation or
                    // an independently confirmed aggregate rollback may move
                    // the projection backwards.
                    return ThreadRootProjectionDecision::Existing(record.clone());
                }
                if record.activity_revision == u64::MAX {
                    record.retired = true;
                    record.aggregate_refresh = None;
                    return ThreadRootProjectionDecision::Retired;
                }
                let previous = record.activity.clone();
                record.activity_revision += 1;
                record.activity = activity.clone();
                record.aggregate_failure = None;
                record.pending_rollback = None;
                if newer {
                    update_live_activity_floor(record, activity, &previous);
                } else {
                    record.live_activity_floor = None;
                }
                return ThreadRootProjectionDecision::ActivityUpdated(record.clone());
            }
            return ThreadRootProjectionDecision::Existing(record.clone());
        }
        if self
            .attempts
            .keys()
            .filter(|(room_id, _)| room_id == &activity.room_id)
            .count()
            >= THREAD_SUMMARY_PROJECTION_MAX_ROOTS
        {
            // The Room window and its projection wake share this bound. A
            // root beyond it has no bounded canonical presentation slot.
            return ThreadRootProjectionDecision::Retired;
        }
        self.attempts.insert(
            key,
            ThreadRootProjectionRecord {
                activity: activity.clone(),
                aggregate: AuthoritativeThreadAggregate::default(),
                activity_revision: 1,
                summary_revision: 0,
                root_item: None,
                aggregate_refresh: None,
                aggregate_failure: None,
                live_activity_floor: Some(LiveActivityFloor {
                    activity: activity.clone(),
                    reply_count: 1,
                }),
                pending_rollback: None,
                invalidated_activity: None,
                retired: false,
                attempt: ThreadRootProjectionAttempt::Pending,
            },
        );
        ThreadRootProjectionDecision::StartFetch(activity)
    }

    /// Apply an observation from the exact Thread timeline. Older activity is
    /// ignored while the current latest live floor is still valid; after a
    /// matching invalidation, the exact SDK aggregate may select the older
    /// reply again.
    pub(crate) fn observe_live_activity(
        &mut self,
        activity: ThreadRootProjectionActivity,
    ) -> ThreadRootProjectionDecision {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        let Some(record) = self.attempts.get(&key) else {
            return self.observe(activity);
        };
        if record.retired || activity == record.activity {
            return self.observe(activity);
        }
        let aggregate_matches = record.live_activity_floor.is_none()
            && record.aggregate.latest_event_id.as_deref()
                == Some(activity.activity_event_id.as_str());
        let invalidated_latest = record
            .invalidated_activity
            .as_ref()
            .is_some_and(|(event_id, _)| event_id == &record.activity.activity_event_id);
        if !activity_is_newer(&activity, &record.activity)
            && !invalidated_latest
            && !aggregate_matches
        {
            return ThreadRootProjectionDecision::Existing(record.clone());
        }
        if invalidated_latest || aggregate_matches {
            let Some(record) = self.attempts.get_mut(&key) else {
                return ThreadRootProjectionDecision::Retired;
            };
            if record.activity_revision == u64::MAX {
                record.retired = true;
                record.aggregate_refresh = None;
                return ThreadRootProjectionDecision::Retired;
            }
            let previous = record.activity.clone();
            let newer = activity_is_newer(&activity, &previous);
            record.activity_revision += 1;
            record.activity = activity.clone();
            record.aggregate_failure = None;
            record.pending_rollback = None;
            if newer {
                update_live_activity_floor(record, activity, &previous);
            } else {
                record.live_activity_floor = None;
            }
            return ThreadRootProjectionDecision::ActivityUpdated(record.clone());
        }
        self.observe(activity)
    }

    #[cfg(test)]
    pub(crate) fn schedule_aggregate_refresh(
        &mut self,
        activity: &ThreadRootProjectionActivity,
        cause: AggregateRefreshCause,
        root_active: bool,
        advance_activity_revision: bool,
    ) -> Option<AggregateRefresh> {
        let canonical_root_active = self
            .canonical_root_event_ids
            .get(&activity.room_id)
            .is_some_and(|roots| roots.contains(&activity.root_event_id));
        self.schedule_aggregate_refresh_with_canonical_root(
            activity,
            cause,
            root_active,
            canonical_root_active,
            advance_activity_revision,
        )
    }

    pub(crate) fn schedule_aggregate_refresh_with_canonical_root(
        &mut self,
        activity: &ThreadRootProjectionActivity,
        cause: AggregateRefreshCause,
        root_active: bool,
        canonical_root_active: bool,
        advance_activity_revision: bool,
    ) -> Option<AggregateRefresh> {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        let record = self.attempts.get_mut(&key)?;
        if record.retired {
            return None;
        }
        if advance_activity_revision {
            if record.activity_revision == u64::MAX {
                record.retired = true;
                record.aggregate_refresh = None;
                return None;
            }
            record.activity_revision += 1;
        }
        if record.summary_revision == u64::MAX {
            record.retired = true;
            record.aggregate_refresh = None;
            return None;
        }
        record.summary_revision += 1;
        record.activity = activity.clone();
        record.aggregate_failure = None;
        if canonical_root_active {
            if matches!(record.attempt, ThreadRootProjectionAttempt::Pending) {
                record.attempt = ThreadRootProjectionAttempt::Canonical;
            }
        } else if matches!(record.attempt, ThreadRootProjectionAttempt::Canonical) {
            record.attempt = ThreadRootProjectionAttempt::Pending;
        }
        let refresh = AggregateRefresh {
            activity: record.activity.clone(),
            activity_revision: record.activity_revision,
            summary_revision: record.summary_revision,
            cause,
            root_active,
            canonical_root_active,
            hydrate_root: !canonical_root_active
                && matches!(record.attempt, ThreadRootProjectionAttempt::Pending),
        };
        record.aggregate_refresh = Some(refresh.clone());
        Some(refresh)
    }

    #[cfg(test)]
    pub(crate) fn pending_refresh(
        &self,
        activity: &ThreadRootProjectionActivity,
    ) -> Option<AggregateRefresh> {
        self.attempts
            .get(&(activity.room_id.clone(), activity.root_event_id.clone()))
            .and_then(ThreadRootProjectionRecord::pending_refresh)
    }

    pub(crate) fn complete_refresh(
        &mut self,
        refresh: &AggregateRefresh,
        result: Result<ThreadRootProjectionRefreshResult, OperationFailureKind>,
    ) -> ThreadRootProjectionCompletion {
        let key = (
            refresh.activity.room_id.clone(),
            refresh.activity.root_event_id.clone(),
        );
        let valid = self.attempts.get(&key).is_some_and(|record| {
            !record.retired
                && record.activity_revision == refresh.activity_revision
                && record.summary_revision == refresh.summary_revision
                && record.aggregate_refresh.as_ref() == Some(refresh)
        });
        if !valid {
            return ThreadRootProjectionCompletion::Ignored;
        }
        let ordinals = self.diagnostic_ordinals.ordinals(&refresh.activity);
        let before;
        let candidate;
        let after;
        let activity;
        let clear;
        let mut merge_reason = "failure";
        let completion = {
            let record = self
                .attempts
                .get_mut(&key)
                .expect("validated thread-root projection record");
            before = record.aggregate.clone();
            record.aggregate_refresh = None;
            match result {
                Ok(ThreadRootProjectionRefreshResult::Hydrated { item, aggregate }) => {
                    candidate = Some(aggregate.clone());
                    record.root_item = Some(item.clone());
                    record.attempt = ThreadRootProjectionAttempt::Ready;
                    merge_reason = merge_aggregate(record, aggregate, false);
                    record.aggregate_failure = None;
                }
                Ok(ThreadRootProjectionRefreshResult::Aggregate(aggregate)) => {
                    candidate = Some(aggregate.clone());
                    merge_reason = merge_aggregate(record, aggregate, false);
                    record.aggregate_failure = None;
                }
                Err(failure_kind) => {
                    candidate = None;
                    record.aggregate_failure = Some(failure_kind);
                }
            }
            activity = record.activity.clone();
            after = record.aggregate.clone();
            clear = after.reply_count == 0
                && !refresh.root_active
                && !refresh.canonical_root_active
                && merge_reason == "invalidation";
            if clear {
                ThreadRootProjectionCompletion::Cleared(activity.clone())
            } else {
                ThreadRootProjectionCompletion::Updated(record.clone())
            }
        };

        if clear {
            self.attempts.remove(&key);
            self.diagnostic_ordinals
                .remove_root(&activity.room_id, &activity.root_event_id);
        }

        let candidate = candidate.as_ref();
        let relation = latest_identity_relation(
            before.latest_event_id.as_deref(),
            candidate.and_then(|candidate| candidate.latest_event_id.as_deref()),
        );
        let source = thread_summary_source(refresh, &before, candidate, merge_reason);
        let decision = if clear
            || ((source == "redaction" || merge_reason == "rollback_confirmed")
                && after.reply_count < before.reply_count)
        {
            "remove"
        } else if candidate.is_some_and(|candidate| *candidate != after) && before == after {
            "retain"
        } else if before == after {
            "no_op"
        } else if before.latest_event_id != after.latest_event_id {
            "advance"
        } else {
            "repair"
        };
        record_thread_summary_reconciliation(
            ordinals,
            source,
            relation,
            decision,
            merge_reason,
            before.reply_count,
            after.reply_count,
            before != after,
        );
        completion
    }

    /// Update Core's current bounded visibility inputs. Visibility is a
    /// scheduling input only; it never deletes an accepted lifecycle record.
    pub(crate) fn reconcile_room_visibility(
        &mut self,
        room_id: &str,
        active_root_event_ids: &HashSet<String>,
    ) {
        self.active_root_event_ids
            .insert(room_id.to_owned(), active_root_event_ids.clone());
    }

    pub(crate) fn reconcile_room_activities(
        &mut self,
        room_id: &str,
        activities_by_root: &HashMap<String, ThreadRootProjectionActivity>,
    ) -> HashSet<String> {
        let active_root_event_ids = activities_by_root.keys().cloned().collect::<HashSet<_>>();
        self.reconcile_room_visibility(room_id, &active_root_event_ids);
        let mut changed = HashSet::new();
        for (root_event_id, activity) in activities_by_root {
            if let Some(record) = self
                .attempts
                .get_mut(&(room_id.to_owned(), root_event_id.clone()))
            {
                if activity != &record.activity {
                    let newer = activity_is_newer(activity, &record.activity);
                    let invalidated_latest =
                        record
                            .invalidated_activity
                            .as_ref()
                            .is_some_and(|(event_id, _)| {
                                event_id == &record.activity.activity_event_id
                            });
                    if !newer && !invalidated_latest {
                        continue;
                    }
                    if record.activity_revision == u64::MAX {
                        record.retired = true;
                        record.aggregate_refresh = None;
                    } else {
                        let previous = record.activity.clone();
                        record.activity_revision += 1;
                        record.activity = activity.clone();
                        record.aggregate_failure = None;
                        record.pending_rollback = None;
                        if newer {
                            update_live_activity_floor(record, activity.clone(), &previous);
                        } else {
                            record.live_activity_floor = None;
                        }
                        changed.insert(root_event_id.clone());
                    }
                }
            }
        }
        changed
    }

    pub(crate) fn set_canonical_root_event_ids(
        &mut self,
        room_id: &str,
        root_event_ids: &HashSet<String>,
    ) {
        self.canonical_root_event_ids
            .insert(room_id.to_owned(), root_event_ids.clone());
    }

    pub(crate) fn canonical_root_active(&self, room_id: &str, root_event_id: &str) -> bool {
        self.canonical_root_event_ids
            .get(room_id)
            .is_some_and(|roots| roots.contains(root_event_id))
    }

    pub(crate) fn activity_active(&self, room_id: &str, root_event_id: &str) -> bool {
        self.active_root_event_ids
            .get(room_id)
            .is_some_and(|roots| roots.contains(root_event_id))
    }

    pub(crate) fn activity_for_root(
        &self,
        room_id: &str,
        root_event_id: &str,
    ) -> Option<ThreadRootProjectionActivity> {
        self.attempts
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .filter(|record| !record.retired)
            .map(|record| record.activity.clone())
    }

    pub(crate) fn invalidate_live_activity(
        &mut self,
        room_id: &str,
        root_event_id: &str,
        activity_event_id: &str,
    ) -> bool {
        let Some(record) = self
            .attempts
            .get_mut(&(room_id.to_owned(), root_event_id.to_owned()))
        else {
            return false;
        };
        let invalidates_latest = record
            .live_activity_floor
            .as_ref()
            .is_some_and(|floor| floor.activity.activity_event_id == activity_event_id)
            || record.activity.activity_event_id == activity_event_id
            || record.aggregate.latest_event_id.as_deref() == Some(activity_event_id);
        if !invalidates_latest {
            return false;
        }
        record.live_activity_floor = None;
        record.pending_rollback = None;
        record.invalidated_activity = Some((
            activity_event_id.to_owned(),
            record.activity.activity_timestamp_ms,
        ));
        true
    }

    pub(crate) fn current_aggregate(
        &self,
        room_id: &str,
        root_event_id: &str,
    ) -> Option<AuthoritativeThreadAggregate> {
        self.attempts
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .filter(|record| !record.retired)
            .map(effective_aggregate)
    }

    pub(crate) fn aggregate_at_revision(
        &self,
        room_id: &str,
        root_event_id: &str,
        activity_revision: u64,
        summary_revision: u64,
    ) -> Option<AuthoritativeThreadAggregate> {
        self.attempts
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .filter(|record| {
                !record.retired
                    && record.activity_revision == activity_revision
                    && record.summary_revision == summary_revision
                    && record.aggregate_refresh.is_none()
                    && !record.is_hydration_pending()
            })
            .map(effective_aggregate)
    }

    pub(crate) fn display_data_for_room(&self, room_id: &str) -> Vec<ThreadRootDisplayData> {
        let mut roots = self
            .attempts
            .iter()
            .filter_map(|((entry_room_id, _), record)| {
                (entry_room_id == room_id && !record.retired).then(|| record.display_data())
            })
            .collect::<Vec<_>>();
        roots.sort_by(|left, right| left.root_event_id.cmp(&right.root_event_id));
        roots
    }

    pub(crate) fn display_data_at_revision(
        &self,
        room_id: &str,
        root_event_id: &str,
        activity_revision: u64,
        summary_revision: u64,
    ) -> Option<ThreadRootDisplayData> {
        self.attempts
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .filter(|record| {
                !record.retired
                    && record.activity_revision == activity_revision
                    && record.summary_revision == summary_revision
            })
            .map(ThreadRootProjectionRecord::display_data)
    }

    pub(crate) fn active_activities(
        &self,
        room_id: &str,
    ) -> HashMap<String, ThreadRootProjectionActivity> {
        self.attempts
            .iter()
            .filter_map(|((entry_room_id, root_event_id), record)| {
                (entry_room_id == room_id && !record.retired)
                    .then(|| (root_event_id.clone(), record.activity.clone()))
            })
            .collect()
    }

    /// Remove all state for a Room when its Room timeline is unsubscribed.
    /// Returning the records lets the owner clear matching frontend snapshots
    /// before a later actor for the same room can be created.
    pub(crate) fn clear_room(&mut self, room_id: &str) -> Vec<ThreadRootProjectionRecord> {
        self.active_root_event_ids.remove(room_id);
        self.canonical_root_event_ids.remove(room_id);
        self.diagnostic_ordinals.clear_room(room_id);
        let keys = self
            .attempts
            .keys()
            .filter(|(entry_room_id, _)| entry_room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| self.attempts.remove(&key))
            .collect()
    }

    pub(crate) fn has_pending_attempt(&self, activity: &ThreadRootProjectionActivity) -> bool {
        self.attempts
            .get(&(activity.room_id.clone(), activity.root_event_id.clone()))
            .is_some_and(ThreadRootProjectionRecord::is_hydration_pending)
    }

    /// Returns a retained terminal record without observing the root again or
    /// starting another bounded lookup. Tests use this to prove that bounded
    /// visibility changes cannot erase terminal lifecycle state.
    #[cfg(test)]
    pub(crate) fn terminal_record(
        &self,
        room_id: &str,
        root_event_id: &str,
    ) -> Option<ThreadRootProjectionRecord> {
        self.attempts
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .filter(|record| !record.is_pending())
            .cloned()
    }

    pub(crate) fn mark_ready(
        &mut self,
        activity: &ThreadRootProjectionActivity,
        item: TimelineItem,
    ) -> Option<ThreadRootProjectionRecord> {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        let record = self.attempts.get_mut(&key)?;
        record.root_item = Some(item.clone());
        record.attempt = ThreadRootProjectionAttempt::Ready;
        Some(record.clone())
    }

    pub(crate) fn mark_failed(
        &mut self,
        activity: &ThreadRootProjectionActivity,
        failure_kind: OperationFailureKind,
    ) -> Option<ThreadRootProjectionRecord> {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        let record = self.attempts.get_mut(&key)?;
        record.attempt = ThreadRootProjectionAttempt::Failed(failure_kind);
        Some(record.clone())
    }
}

/// Same-ID edits are newer activity, while a different older reply must not
/// replace the accepted latest live activity.
pub(crate) fn activity_is_newer(
    candidate: &ThreadRootProjectionActivity,
    existing: &ThreadRootProjectionActivity,
) -> bool {
    if candidate == existing {
        return false;
    }
    if candidate.activity_event_id == existing.activity_event_id {
        return true;
    }
    match (
        candidate.activity_timestamp_ms,
        existing.activity_timestamp_ms,
    ) {
        (Some(candidate), Some(existing)) if candidate != existing => candidate > existing,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => candidate.activity_event_id > existing.activity_event_id,
        (Some(_), Some(_)) => candidate.activity_event_id > existing.activity_event_id,
    }
}

fn update_live_activity_floor(
    record: &mut ThreadRootProjectionRecord,
    activity: ThreadRootProjectionActivity,
    previous: &ThreadRootProjectionActivity,
) {
    // A newer renderable activity supersedes any rollback intent for the
    // previously invalidated latest event.
    if record
        .invalidated_activity
        .as_ref()
        .is_none_or(|(event_id, _)| event_id != &activity.activity_event_id)
    {
        record.invalidated_activity = None;
    }
    let base_count = record.aggregate.reply_count;
    let floor_count = record
        .live_activity_floor
        .as_ref()
        .map(|floor| floor.reply_count)
        .unwrap_or(0);
    let aggregate_already_includes_activity =
        record.aggregate.latest_event_id.as_deref() == Some(activity.activity_event_id.as_str());
    let reply_count = if aggregate_already_includes_activity
        || activity.activity_event_id == previous.activity_event_id
    {
        floor_count.max(base_count).max(1)
    } else {
        floor_count
            .saturating_add(1)
            .max(base_count.saturating_add(1))
            .max(1)
    };
    record.live_activity_floor = Some(LiveActivityFloor {
        activity,
        reply_count,
    });
}

fn effective_aggregate(record: &ThreadRootProjectionRecord) -> AuthoritativeThreadAggregate {
    let mut aggregate = record.aggregate.clone();
    if let Some(floor) = &record.live_activity_floor {
        merge_floor_into_aggregate(&mut aggregate, floor);
    }
    aggregate
}

fn merge_aggregate(
    record: &mut ThreadRootProjectionRecord,
    candidate: AuthoritativeThreadAggregate,
    force_disappearance: bool,
) -> &'static str {
    let rollback_confirmed = record.pending_rollback.take().is_some_and(|pending| {
        pending.reply_count == candidate.reply_count
            && pending.latest_event_id == candidate.latest_event_id
            && pending.latest_timestamp_ms == candidate.latest_timestamp_ms
    });
    let (invalidation_confirmed, invalidation_superseded) =
        record.invalidated_activity.as_ref().map_or(
            (false, false),
            |(invalidated_event_id, invalidated_timestamp)| {
                if candidate.latest_event_id.as_deref() == Some(invalidated_event_id.as_str()) {
                    return (false, false);
                }
                let candidate_is_newer =
                    match (candidate.latest_timestamp_ms, *invalidated_timestamp) {
                        (Some(candidate), Some(invalidated)) if candidate != invalidated => {
                            candidate > invalidated
                        }
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                        (None, None) | (Some(_), Some(_)) => candidate
                            .latest_event_id
                            .as_deref()
                            .is_some_and(|candidate| candidate > invalidated_event_id.as_str()),
                    };
                (!candidate_is_newer, candidate_is_newer)
            },
        );
    let force = force_disappearance || invalidation_confirmed || rollback_confirmed;
    if invalidation_confirmed || invalidation_superseded {
        record.invalidated_activity = None;
    }
    if (rollback_confirmed || invalidation_confirmed)
        && let Some(event_id) = candidate.latest_event_id.as_ref()
    {
        let rollback_activity = ThreadRootProjectionActivity {
            room_id: record.activity.room_id.clone(),
            root_event_id: record.activity.root_event_id.clone(),
            activity_event_id: event_id.clone(),
            activity_timestamp_ms: candidate.latest_timestamp_ms,
            activity_sender: candidate.latest_sender.clone(),
            activity_sender_label: candidate.latest_sender_label.clone(),
            activity_body_preview: candidate.latest_body_preview.clone(),
        };
        if rollback_activity != record.activity {
            if record.activity_revision == u64::MAX {
                record.retired = true;
                record.aggregate_refresh = None;
            } else {
                record.activity_revision += 1;
                record.activity = rollback_activity;
            }
        }
    }
    let floor = record.live_activity_floor.clone();
    let mut aggregate = if force {
        candidate
    } else {
        merge_sdk_aggregate(&record.aggregate, &candidate)
    };
    if let Some(floor) = floor {
        if force {
            // An explicit redaction/removal restores the exact SDK aggregate,
            // including a zero-count result, instead of retaining the floor.
            record.live_activity_floor = None;
        } else if aggregate_latest_is_at_least(&aggregate, &floor.activity) {
            aggregate.reply_count = aggregate.reply_count.max(floor.reply_count);
            record.live_activity_floor = None;
        } else {
            merge_floor_into_aggregate(&mut aggregate, &floor);
            record.live_activity_floor = Some(floor);
        }
    }
    record.aggregate = aggregate;
    if rollback_confirmed {
        "rollback_confirmed"
    } else if force {
        "invalidation"
    } else {
        "normal"
    }
}

fn merge_floor_into_aggregate(
    aggregate: &mut AuthoritativeThreadAggregate,
    floor: &LiveActivityFloor,
) {
    aggregate.reply_count = aggregate.reply_count.max(floor.reply_count);
    aggregate.latest_event_id = Some(floor.activity.activity_event_id.clone());
    aggregate.latest_sender = floor.activity.activity_sender.clone();
    aggregate.latest_sender_label = floor.activity.activity_sender_label.clone();
    aggregate.latest_body_preview = floor.activity.activity_body_preview.clone();
    aggregate.latest_timestamp_ms = floor.activity.activity_timestamp_ms;
}

fn merge_sdk_aggregate(
    previous: &AuthoritativeThreadAggregate,
    candidate: &AuthoritativeThreadAggregate,
) -> AuthoritativeThreadAggregate {
    let Some(candidate_event_id) = candidate.latest_event_id.as_deref() else {
        let mut retained = previous.clone();
        retained.reply_count = retained.reply_count.max(candidate.reply_count);
        return retained;
    };
    let Some(previous_event_id) = previous.latest_event_id.as_deref() else {
        return candidate.clone();
    };
    if candidate_event_id == previous_event_id {
        let mut repaired = candidate.clone();
        repaired.reply_count = repaired.reply_count.max(previous.reply_count);
        return repaired;
    }
    let candidate_is_newer = match (candidate.latest_timestamp_ms, previous.latest_timestamp_ms) {
        (Some(candidate), Some(previous)) if candidate != previous => candidate > previous,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => candidate_event_id > previous_event_id,
        (Some(_), Some(_)) => candidate_event_id > previous_event_id,
    };
    if candidate_is_newer {
        let mut advanced = candidate.clone();
        advanced.reply_count = advanced.reply_count.max(previous.reply_count);
        advanced
    } else {
        previous.clone()
    }
}

fn aggregate_latest_is_at_least(
    aggregate: &AuthoritativeThreadAggregate,
    activity: &ThreadRootProjectionActivity,
) -> bool {
    let Some(event_id) = aggregate.latest_event_id.as_deref() else {
        return false;
    };
    if event_id == activity.activity_event_id {
        return true;
    }
    match (
        aggregate.latest_timestamp_ms,
        activity.activity_timestamp_ms,
    ) {
        (Some(aggregate), Some(activity)) if aggregate != activity => aggregate > activity,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => event_id > activity.activity_event_id.as_str(),
        (Some(_), Some(_)) => event_id > activity.activity_event_id.as_str(),
    }
}

fn latest_identity_relation(previous: Option<&str>, candidate: Option<&str>) -> &'static str {
    match (previous, candidate) {
        (None, None) => "missing",
        (Some(previous), Some(candidate)) if previous == candidate => "same",
        _ => "different",
    }
}

fn thread_summary_source(
    refresh: &AggregateRefresh,
    previous: &AuthoritativeThreadAggregate,
    candidate: Option<&AuthoritativeThreadAggregate>,
    merge_reason: &'static str,
) -> &'static str {
    match refresh.cause {
        AggregateRefreshCause::InitialHydration => "rehydration",
        AggregateRefreshCause::SelectedActivity => {
            let candidate_event_id =
                candidate.and_then(|candidate| candidate.latest_event_id.as_deref());
            if candidate_event_id == Some(refresh.activity.activity_event_id.as_str())
                && previous.latest_event_id.as_deref()
                    == Some(refresh.activity.activity_event_id.as_str())
            {
                "edit"
            } else {
                "live_reply"
            }
        }
        AggregateRefreshCause::CanonicalBatch if merge_reason == "invalidation" => "redaction",
        AggregateRefreshCause::CanonicalBatch => "sdk_summary",
        AggregateRefreshCause::Removal => "redaction",
    }
}

/// Messages routed to a `ThreadsListActor`.
pub enum ThreadsListMessage {
    Open {
        request_id: RequestId,
        scope: ThreadsListScope,
        room_ids: Vec<String>,
    },
    Close {
        request_id: RequestId,
    },
    Paginate {
        request_id: RequestId,
    },
    Shutdown,
}

/// Handle to a `ThreadsListActor` background task.
pub struct ThreadsListActorHandle {
    tx: mpsc::Sender<ThreadsListMessage>,
    task: Option<executor::JoinHandle<()>>,
}

impl ThreadsListActorHandle {
    pub async fn open(
        &self,
        request_id: RequestId,
        scope: ThreadsListScope,
        room_ids: Vec<String>,
    ) -> bool {
        self.tx
            .send(ThreadsListMessage::Open {
                request_id,
                scope,
                room_ids,
            })
            .await
            .is_ok()
    }

    pub async fn close(mut self, request_id: RequestId) -> bool {
        let closed = matches!(
            executor::timeout(
                THREADS_LIST_SHUTDOWN_SEND_TIMEOUT,
                self.tx.send(ThreadsListMessage::Close { request_id }),
            )
            .await,
            Ok(Ok(()))
        );
        let shutdown = self.shutdown_inner().await;
        closed && shutdown
    }

    pub async fn paginate(&self, request_id: RequestId) -> bool {
        self.tx
            .send(ThreadsListMessage::Paginate { request_id })
            .await
            .is_ok()
    }

    pub async fn shutdown(mut self) -> bool {
        self.shutdown_inner().await
    }

    async fn shutdown_inner(&mut self) -> bool {
        let sent = matches!(
            executor::timeout(
                THREADS_LIST_SHUTDOWN_SEND_TIMEOUT,
                self.tx.send(ThreadsListMessage::Shutdown),
            )
            .await,
            Ok(Ok(()))
        );
        let Some(mut task) = self.task.take() else {
            return sent;
        };
        if sent
            && executor::timeout(THREADS_LIST_SHUTDOWN_JOIN_TIMEOUT, &mut task)
                .await
                .is_ok()
        {
            return true;
        }
        task.abort();
        let _ = task.await;
        false
    }
}

impl Drop for ThreadsListActorHandle {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub struct ThreadsListActor {
    session: Arc<koushi_sdk::MatrixClientSession>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    msg_rx: mpsc::Receiver<ThreadsListMessage>,
}

impl ThreadsListActor {
    pub fn spawn(
        session: Arc<koushi_sdk::MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
    ) -> ThreadsListActorHandle {
        let (tx, msg_rx) = mpsc::channel(16);
        let actor = ThreadsListActor {
            session,
            action_tx,
            event_tx,
            msg_rx,
        };
        let task = executor::spawn(actor.run());
        ThreadsListActorHandle {
            tx,
            task: Some(task),
        }
    }

    async fn run(mut self) {
        let mut active: Option<ActiveSubscription> = None;
        while let Some(msg) = self.msg_rx.recv().await {
            match msg {
                ThreadsListMessage::Shutdown | ThreadsListMessage::Close { .. } => {
                    if let Some(subscription) = active.take() {
                        subscription.shutdown().await;
                    }
                    if matches!(msg, ThreadsListMessage::Shutdown) {
                        break;
                    }
                }
                ThreadsListMessage::Open {
                    request_id,
                    scope,
                    room_ids,
                } => {
                    if let Some(subscription) = active.take() {
                        subscription.shutdown().await;
                    }
                    active = self.open_subscription(request_id, scope, room_ids).await;
                }
                ThreadsListMessage::Paginate { request_id } => {
                    if let Some(sub) = active.as_ref() {
                        sub.paginate(request_id).await;
                    }
                }
            }
        }
        if let Some(subscription) = active {
            subscription.shutdown().await;
        }
    }

    async fn open_subscription(
        &self,
        request_id: RequestId,
        scope: ThreadsListScope,
        room_ids: Vec<String>,
    ) -> Option<ActiveSubscription> {
        let mut services = BTreeMap::new();
        for room_id in room_ids {
            let room_id_value = match RoomId::parse(room_id.as_str()) {
                Ok(id) => id,
                Err(_) => {
                    self.emit_failed(&scope, request_id, OperationFailureKind::Invalid)
                        .await;
                    return None;
                }
            };
            let room = match self.session.client().get_room(&room_id_value) {
                Some(room) => room,
                None => {
                    self.emit_failed(&scope, request_id, OperationFailureKind::NotFound)
                        .await;
                    return None;
                }
            };
            services.insert(room_id, Arc::new(ThreadListService::new(room)));
        }

        let item_subscribers = services
            .iter()
            .map(|(room_id, service)| {
                let (_, subscriber) = service.subscribe_to_items_updates();
                (room_id.clone(), Arc::clone(service), subscriber)
            })
            .collect::<Vec<_>>();
        let (items_tx, mut items_rx) = mpsc::channel(64);
        let (pagination_tx, mut pagination_rx) = mpsc::channel(16);
        let (pagination_request_tx, mut pagination_request_rx) = mpsc::channel(16);
        let (pagination_failure_tx, mut pagination_failure_rx) = mpsc::channel(16);

        let items_relay_handles = item_subscribers
            .into_iter()
            .map(|(room_id, service, mut subscriber)| {
                let items_tx = items_tx.clone();
                executor::spawn(async move {
                    loop {
                        match subscriber.next().await {
                            Some(_) => {
                                if items_tx.send(room_id.clone()).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    drop(service);
                })
            })
            .collect::<Vec<_>>();

        let pagination_relay_handles = services
            .iter()
            .map(|(room_id, service)| {
                let room_id = room_id.clone();
                let pagination_tx = pagination_tx.clone();
                let mut subscriber = service.subscribe_to_pagination_state_updates();
                executor::spawn(async move {
                    while let Some(state) = subscriber.next().await {
                        if pagination_tx.send((room_id.clone(), state)).await.is_err() {
                            break;
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut tasks = SubscriptionTasks::new(
            items_relay_handles
                .into_iter()
                .chain(pagination_relay_handles)
                .collect(),
        );
        drop(items_tx);
        drop(pagination_tx);

        let initial_results =
            join_all(services.iter().map(|(room_id, service)| async move {
                (room_id.clone(), service.paginate().await)
            }))
            .await;
        if initial_results.iter().any(|(_, result)| result.is_err()) {
            tasks.shutdown().await;
            self.emit_failed(&scope, request_id, OperationFailureKind::Sdk)
                .await;
            return None;
        }
        let projected = projected_items(&services);
        let initial_end_reached = end_reached(&services);
        self.emit_opened(&scope, request_id, projected, initial_end_reached)
            .await;

        let action_tx = self.action_tx.clone();
        let event_tx = self.event_tx.clone();
        let update_services = services.clone();
        let update_scope = scope.clone();
        let update_task = executor::spawn(async move {
            let mut current_request_id = request_id;
            let mut failed_pagination_request_id: Option<u64> = None;
            loop {
                tokio::select! {
                    biased;
                    Some(next_request_id) = pagination_request_rx.recv() => {
                        current_request_id = next_request_id;
                    }
                    Some((failed_request_id, failure_kind)) = pagination_failure_rx.recv() => {
                        current_request_id = failed_request_id;
                        failed_pagination_request_id = Some(failed_request_id.sequence);
                        let scope_key = update_scope.scope_key();
                        let _ = action_tx.send(vec![AppAction::ThreadsListFailed {
                            request_id: failed_request_id.sequence,
                            room_id: scope_key.clone(),
                            failure_kind,
                        }]).await;
                        let _ = event_tx.send(CoreEvent::ThreadsList(ThreadsListEvent::Failed {
                            request_id: failed_request_id,
                            room_id: scope_key,
                            failure_kind,
                        }));
                    }
                    Some(_) = items_rx.recv() => {
                        let projected = projected_items(&update_services);
                        let scope_key = update_scope.scope_key();
                        let _ = action_tx.send(vec![AppAction::ThreadsListUpdated {
                            request_id: current_request_id.sequence,
                            room_id: scope_key.clone(),
                            items: projected.clone(),
                            is_paginating: false,
                            end_reached: crate::threads_list::end_reached(&update_services),
                        }]).await;
                        let _ = event_tx.send(CoreEvent::ThreadsList(ThreadsListEvent::Updated {
                            request_id: current_request_id,
                            room_id: scope_key,
                            items: projected,
                            is_paginating: false,
                            end_reached: crate::threads_list::end_reached(&update_services),
                        }));
                    }
                    Some((_, _state)) = pagination_rx.recv() => {
                        let projected = projected_items(&update_services);
                        let is_paginating = update_services.values().any(|service| {
                            matches!(service.pagination_state(), ThreadListPaginationState::Loading)
                        });
                        let end_reached = crate::threads_list::end_reached(&update_services);
                        if !is_paginating && failed_pagination_request_id == Some(current_request_id.sequence) {
                            failed_pagination_request_id = None;
                            continue;
                        }
                        if is_paginating {
                            failed_pagination_request_id = None;
                        }
                        let action = if is_paginating {
                            AppAction::ThreadsListUpdated {
                                request_id: current_request_id.sequence,
                                room_id: update_scope.scope_key(),
                                items: projected.clone(),
                                is_paginating: true,
                                end_reached,
                            }
                        } else {
                            AppAction::ThreadsListPaginationCompleted {
                                request_id: current_request_id.sequence,
                                room_id: update_scope.scope_key(),
                                items: projected.clone(),
                                end_reached,
                            }
                        };
                        let _ = action_tx.send(vec![action]).await;
                        let event = if is_paginating {
                            CoreEvent::ThreadsList(ThreadsListEvent::Updated {
                                request_id: current_request_id,
                                room_id: update_scope.scope_key(),
                                items: projected.clone(),
                                is_paginating: true,
                                end_reached,
                            })
                        } else {
                            CoreEvent::ThreadsList(ThreadsListEvent::PaginationCompleted {
                                request_id: current_request_id,
                                room_id: update_scope.scope_key(),
                                items: projected,
                                end_reached,
                            })
                        };
                        let _ = event_tx.send(event);
                    }
                    else => break,
                }
            }
        });

        tasks.push(update_task);
        Some(ActiveSubscription {
            services,
            pagination_request_tx,
            pagination_failure_tx,
            tasks,
        })
    }

    async fn emit_opened(
        &self,
        scope: &ThreadsListScope,
        request_id: RequestId,
        items: Vec<ThreadsListItem>,
        end_reached: bool,
    ) {
        let room_id = scope.scope_key();
        let _ = self
            .action_tx
            .send(vec![AppAction::ThreadsListOpened {
                request_id: request_id.sequence,
                room_id: room_id.clone(),
                items: items.clone(),
                end_reached,
            }])
            .await;
        let _ = self
            .event_tx
            .send(CoreEvent::ThreadsList(ThreadsListEvent::Opened {
                request_id,
                room_id,
                items,
                end_reached,
            }));
    }

    async fn emit_failed(
        &self,
        scope: &ThreadsListScope,
        request_id: RequestId,
        failure_kind: OperationFailureKind,
    ) {
        let room_id = scope.scope_key();
        let _ = self
            .action_tx
            .send(vec![AppAction::ThreadsListFailed {
                request_id: request_id.sequence,
                room_id: room_id.clone(),
                failure_kind,
            }])
            .await;
        let _ = self
            .event_tx
            .send(CoreEvent::ThreadsList(ThreadsListEvent::Failed {
                request_id,
                room_id,
                failure_kind,
            }));
    }
}

struct ActiveSubscription {
    services: BTreeMap<String, Arc<ThreadListService>>,
    pagination_request_tx: mpsc::Sender<RequestId>,
    pagination_failure_tx: mpsc::Sender<(RequestId, OperationFailureKind)>,
    tasks: SubscriptionTasks,
}

impl ActiveSubscription {
    async fn paginate(&self, request_id: RequestId) {
        if self.pagination_request_tx.send(request_id).await.is_err() {
            return;
        }
        let results = join_all(self.services.values().map(|service| service.paginate())).await;
        if let Some(error) = results.into_iter().find_map(Result::err) {
            let _ = self
                .pagination_failure_tx
                .send((request_id, classify_thread_list_error(&error)))
                .await;
        }
    }

    async fn shutdown(mut self) {
        self.tasks.shutdown().await;
    }
}

struct SubscriptionTasks {
    tasks: Vec<executor::JoinHandle<()>>,
}

impl SubscriptionTasks {
    fn new(tasks: Vec<executor::JoinHandle<()>>) -> Self {
        Self { tasks }
    }

    fn push(&mut self, task: executor::JoinHandle<()>) {
        self.tasks.push(task);
    }

    async fn shutdown(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }
}

impl Drop for SubscriptionTasks {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

pub(crate) fn classify_thread_list_error(error: &ThreadListServiceError) -> OperationFailureKind {
    match error {
        ThreadListServiceError::Sdk(matrix_sdk::Error::Http(_)) => OperationFailureKind::Network,
        ThreadListServiceError::Sdk(_) | ThreadListServiceError::EventCache(_) => {
            OperationFailureKind::Sdk
        }
    }
}

fn project_item(room_id: &str, item: &SdkThreadListItem) -> ThreadsListItem {
    ThreadsListItem {
        room_id: room_id.to_owned(),
        root_event_id: item.root_event.event_id.to_string(),
        root_sender: item.root_event.sender.to_string(),
        root_sender_label: sender_label(&item.root_event.sender_profile),
        root_body_preview: body_preview(item.root_event.content.as_ref()),
        root_timestamp_ms: Some(item.root_event.timestamp.0.into()),
        latest_event_id: item.latest_event.as_ref().map(|e| e.event_id.to_string()),
        latest_sender: item.latest_event.as_ref().map(|e| e.sender.to_string()),
        latest_sender_label: item
            .latest_event
            .as_ref()
            .and_then(|e| sender_label(&e.sender_profile)),
        latest_body_preview: item
            .latest_event
            .as_ref()
            .and_then(|e| body_preview(e.content.as_ref())),
        latest_timestamp_ms: item.latest_event.as_ref().map(|e| e.timestamp.0.into()),
        reply_count: item.num_replies,
    }
}

fn projected_items(services: &BTreeMap<String, Arc<ThreadListService>>) -> Vec<ThreadsListItem> {
    let mut seen = HashSet::new();
    let mut projected = Vec::new();
    for (room_id, service) in services {
        for item in service.items() {
            let item = project_item(room_id, &item);
            if seen.insert((item.room_id.clone(), item.root_event_id.clone())) {
                projected.push(item);
            }
        }
    }
    projected
}

fn end_reached(services: &BTreeMap<String, Arc<ThreadListService>>) -> bool {
    services.values().all(|service| {
        matches!(
            service.pagination_state(),
            ThreadListPaginationState::Idle { end_reached: true }
        )
    })
}

fn sender_label(profile: &TimelineDetails<matrix_sdk_ui::timeline::Profile>) -> Option<String> {
    match profile {
        TimelineDetails::Ready(profile) => profile.display_name.clone(),
        _ => None,
    }
}

fn body_preview(content: Option<&matrix_sdk_ui::timeline::TimelineItemContent>) -> Option<String> {
    if let Some(message) = content.and_then(|c| c.as_message()) {
        return Some(message.body().to_owned());
    }
    if let Some(sticker) = content.and_then(|c| c.as_sticker()) {
        return Some(sticker.content().body.clone());
    }
    None
}

/// Maps the SDK's proven relation aggregate without rebuilding relation
/// semantics in Core. The event remains the SDK's original reply identity;
/// only its already-effective content is used for the preview.
pub(crate) fn authoritative_thread_aggregate_from_sdk(
    aggregate: &ThreadRelationAggregate,
) -> AuthoritativeThreadAggregate {
    let latest = aggregate.latest_event.as_ref();
    AuthoritativeThreadAggregate {
        reply_count: aggregate.num_replies,
        latest_event_id: latest.map(|event| event.event_id.to_string()),
        latest_sender: latest.map(|event| event.sender.to_string()),
        latest_sender_label: latest.and_then(|event| sender_label(&event.sender_profile)),
        latest_body_preview: latest.and_then(|event| body_preview(event.content.as_ref())),
        latest_timestamp_ms: latest.map(|event| event.timestamp.0.into()),
    }
}

#[cfg(test)]
mod tests;
