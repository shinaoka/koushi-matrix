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

use crate::event::{CoreEvent, ThreadsListEvent, TimelineItem};
use crate::executor;
use crate::ids::RequestId;

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

impl AggregateRefreshCause {
    fn is_disappearance(self) -> bool {
        matches!(self, Self::Removal)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AggregateRefresh {
    pub activity: ThreadRootProjectionActivity,
    pub activity_revision: u64,
    pub summary_revision: u64,
    pub cause: AggregateRefreshCause,
    pub root_active: bool,
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
    Ready(TimelineItem),
    Failed(OperationFailureKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ThreadRootProjectionRecord {
    pub activity: ThreadRootProjectionActivity,
    pub aggregate: AuthoritativeThreadAggregate,
    pub activity_revision: u64,
    pub summary_revision: u64,
    aggregate_refresh: Option<AggregateRefresh>,
    aggregate_failure: Option<OperationFailureKind>,
    pub retired: bool,
    attempt: ThreadRootProjectionAttempt,
}

impl ThreadRootProjectionRecord {
    pub(crate) fn item(&self) -> Option<&TimelineItem> {
        match &self.attempt {
            ThreadRootProjectionAttempt::Ready(item) => Some(item),
            ThreadRootProjectionAttempt::Pending | ThreadRootProjectionAttempt::Failed(_) => None,
        }
    }

    pub(crate) fn failure_kind(&self) -> Option<OperationFailureKind> {
        self.aggregate_failure.or_else(|| match self.attempt {
            ThreadRootProjectionAttempt::Failed(kind) => Some(kind),
            ThreadRootProjectionAttempt::Pending | ThreadRootProjectionAttempt::Ready(_) => None,
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
}

impl ThreadRootProjectionService {
    pub(crate) fn observe(
        &mut self,
        activity: ThreadRootProjectionActivity,
    ) -> ThreadRootProjectionDecision {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        if let Some(record) = self.attempts.get_mut(&key) {
            if record.retired {
                return ThreadRootProjectionDecision::Retired;
            }
            if activity_is_newer(&activity, &record.activity) {
                if record.activity_revision == u64::MAX {
                    record.retired = true;
                    record.aggregate_refresh = None;
                    return ThreadRootProjectionDecision::Retired;
                }
                record.activity_revision += 1;
                record.activity = activity;
                record.aggregate_failure = None;
                return ThreadRootProjectionDecision::ActivityUpdated(record.clone());
            }
            return ThreadRootProjectionDecision::Existing(record.clone());
        }
        self.attempts.insert(
            key,
            ThreadRootProjectionRecord {
                activity: activity.clone(),
                aggregate: AuthoritativeThreadAggregate::default(),
                activity_revision: 1,
                summary_revision: 0,
                aggregate_refresh: None,
                aggregate_failure: None,
                retired: false,
                attempt: ThreadRootProjectionAttempt::Pending,
            },
        );
        ThreadRootProjectionDecision::StartFetch(activity)
    }

    pub(crate) fn schedule_aggregate_refresh(
        &mut self,
        activity: &ThreadRootProjectionActivity,
        cause: AggregateRefreshCause,
        root_active: bool,
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
        let refresh = AggregateRefresh {
            activity: record.activity.clone(),
            activity_revision: record.activity_revision,
            summary_revision: record.summary_revision,
            cause,
            root_active,
            hydrate_root: matches!(record.attempt, ThreadRootProjectionAttempt::Pending),
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
        let Some(record) = self.attempts.get_mut(&key) else {
            return ThreadRootProjectionCompletion::Ignored;
        };
        if record.retired
            || record.activity_revision != refresh.activity_revision
            || record.summary_revision != refresh.summary_revision
            || record.aggregate_refresh.as_ref() != Some(refresh)
        {
            return ThreadRootProjectionCompletion::Ignored;
        }
        record.aggregate_refresh = None;
        match result {
            Ok(ThreadRootProjectionRefreshResult::Hydrated { item, aggregate }) => {
                record.attempt = ThreadRootProjectionAttempt::Ready(item);
                record.aggregate = aggregate;
                record.aggregate_failure = None;
                if record.aggregate.reply_count == 0 && !refresh.root_active {
                    let activity = record.activity.clone();
                    self.attempts.remove(&key);
                    self.cleanup_empty_room_tracking(&activity.room_id);
                    ThreadRootProjectionCompletion::Cleared(activity)
                } else {
                    ThreadRootProjectionCompletion::Updated(record.clone())
                }
            }
            Ok(ThreadRootProjectionRefreshResult::Aggregate(aggregate)) => {
                record.aggregate = aggregate;
                record.aggregate_failure = None;
                if record.aggregate.reply_count == 0 && !refresh.root_active {
                    let activity = record.activity.clone();
                    self.attempts.remove(&key);
                    self.cleanup_empty_room_tracking(&activity.room_id);
                    ThreadRootProjectionCompletion::Cleared(activity)
                } else {
                    ThreadRootProjectionCompletion::Updated(record.clone())
                }
            }
            Err(failure_kind) => {
                if !refresh.root_active && refresh.cause.is_disappearance() {
                    let activity = record.activity.clone();
                    self.attempts.remove(&key);
                    self.cleanup_empty_room_tracking(&activity.room_id);
                    ThreadRootProjectionCompletion::Cleared(activity)
                } else {
                    record.aggregate_failure = Some(failure_kind);
                    ThreadRootProjectionCompletion::Updated(record.clone())
                }
            }
        }
    }

    /// Keep only projection data that still has a representation in the
    /// bounded canonical Room window. Pending requests are retained until
    /// their one worker completes; terminal records are dropped as soon as the
    /// corresponding root has no live reply. Thus a reconnect can dedupe a
    /// currently-active failure, while a later observation after cleanup is a
    /// new bounded attempt rather than a retry loop.
    #[cfg(test)]
    pub(crate) fn reconcile_room(
        &mut self,
        room_id: &str,
        active_root_event_ids: &HashSet<String>,
    ) {
        self.reconcile_room_with_affected(room_id, active_root_event_ids, &HashSet::new());
    }

    pub(crate) fn reconcile_room_with_affected(
        &mut self,
        room_id: &str,
        active_root_event_ids: &HashSet<String>,
        affected_root_event_ids: &HashSet<String>,
    ) {
        self.active_root_event_ids
            .insert(room_id.to_owned(), active_root_event_ids.clone());
        self.attempts
            .retain(|(entry_room_id, root_event_id), record| {
                entry_room_id != room_id
                    || active_root_event_ids.contains(root_event_id)
                    || affected_root_event_ids.contains(root_event_id)
                    || record.is_pending()
                    || record.retired
            });
        self.cleanup_empty_room_tracking(room_id);
    }

    #[cfg(test)]
    pub(crate) fn reconcile_room_activities(
        &mut self,
        room_id: &str,
        activities_by_root: &HashMap<String, ThreadRootProjectionActivity>,
    ) -> HashSet<String> {
        self.reconcile_room_activities_with_affected(room_id, activities_by_root, &HashSet::new())
    }

    pub(crate) fn reconcile_room_activities_with_affected(
        &mut self,
        room_id: &str,
        activities_by_root: &HashMap<String, ThreadRootProjectionActivity>,
        affected_root_event_ids: &HashSet<String>,
    ) -> HashSet<String> {
        let active_root_event_ids = activities_by_root.keys().cloned().collect::<HashSet<_>>();
        self.reconcile_room_with_affected(room_id, &active_root_event_ids, affected_root_event_ids);
        let mut changed = HashSet::new();
        for (root_event_id, activity) in activities_by_root {
            if let Some(record) = self
                .attempts
                .get_mut(&(room_id.to_owned(), root_event_id.clone()))
            {
                if activity_is_newer(activity, &record.activity) {
                    if record.activity_revision == u64::MAX {
                        record.retired = true;
                        record.aggregate_refresh = None;
                    } else {
                        record.activity_revision += 1;
                        record.activity = activity.clone();
                        record.aggregate_failure = None;
                        changed.insert(root_event_id.clone());
                    }
                }
            }
        }
        changed
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

    /// Returns the current terminal result for one active root without
    /// observing it again or starting another bounded lookup. Replay-known
    /// ownership uses this only to hand a previously suppressed terminal
    /// snapshot back to the exact canonical reply slot when that owner ends.
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
        let is_active = self.is_active_or_unreported(&activity.room_id, &activity.root_event_id);
        let record = self.attempts.get_mut(&key)?;
        record.attempt = ThreadRootProjectionAttempt::Ready(item);
        let completed = record.clone();
        if !is_active {
            // The UI/state still need this one terminal notification to clear
            // their pending placeholder. The returned snapshot is never
            // retained by this service because its reply already left the
            // canonical window.
            self.attempts.remove(&key);
            self.cleanup_empty_room_tracking(&activity.room_id);
        }
        Some(completed)
    }

    pub(crate) fn mark_failed(
        &mut self,
        activity: &ThreadRootProjectionActivity,
        failure_kind: OperationFailureKind,
    ) -> Option<ThreadRootProjectionRecord> {
        let key = (activity.room_id.clone(), activity.root_event_id.clone());
        let is_active = self.is_active_or_unreported(&activity.room_id, &activity.root_event_id);
        let record = self.attempts.get_mut(&key)?;
        record.attempt = ThreadRootProjectionAttempt::Failed(failure_kind);
        let completed = record.clone();
        if !is_active {
            // See `mark_ready`: terminal completion doubles as the explicit
            // cleanup signal for the independent state/frontend maps.
            self.attempts.remove(&key);
            self.cleanup_empty_room_tracking(&activity.room_id);
        }
        Some(completed)
    }

    fn is_active_or_unreported(&self, room_id: &str, root_event_id: &str) -> bool {
        self.active_root_event_ids
            .get(room_id)
            .is_none_or(|active| active.contains(root_event_id))
    }

    fn cleanup_empty_room_tracking(&mut self, room_id: &str) {
        let has_pending_or_active_record = self
            .attempts
            .keys()
            .any(|(entry_room_id, _)| entry_room_id == room_id);
        if self
            .active_root_event_ids
            .get(room_id)
            .is_some_and(HashSet::is_empty)
            && !has_pending_or_active_record
        {
            self.active_root_event_ids.remove(room_id);
        }
    }
}

/// Effective-content equality, not timestamp/ID ordering, is the activity
/// revision boundary. Same-ID/same-timestamp edits are therefore changes.
pub(crate) fn activity_is_newer(
    candidate: &ThreadRootProjectionActivity,
    existing: &ThreadRootProjectionActivity,
) -> bool {
    candidate != existing
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
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, event_id, user_id};
    use matrix_sdk_ui::timeline::thread_list_service::{
        ThreadListItemEvent, ThreadRelationAggregate,
    };
    use matrix_sdk_ui::timeline::{Profile, TimelineDetails};
    use tokio::sync::{mpsc, oneshot};

    use crate::event::{TimelineItem, TimelineItemId, TimelineMessageActions};

    use super::{
        ActiveSubscription, AggregateRefreshCause, OperationFailureKind, SubscriptionTasks,
        ThreadRootProjectionActivity, ThreadRootProjectionDecision,
        ThreadRootProjectionRefreshResult, ThreadRootProjectionService,
        authoritative_thread_aggregate_from_sdk,
    };

    fn pending_task(settled: oneshot::Sender<()>) -> crate::executor::JoinHandle<()> {
        crate::executor::spawn(async move {
            let _settled = settled;
            std::future::pending::<()>().await;
        })
    }

    fn pending_subscription() -> (ActiveSubscription, [oneshot::Receiver<()>; 3]) {
        let (item_settled_tx, item_settled_rx) = oneshot::channel::<()>();
        let (pagination_settled_tx, pagination_settled_rx) = oneshot::channel::<()>();
        let (update_settled_tx, update_settled_rx) = oneshot::channel::<()>();
        let (pagination_request_tx, _pagination_request_rx) = mpsc::channel(1);
        let (pagination_failure_tx, _pagination_failure_rx) = mpsc::channel(1);
        (
            ActiveSubscription {
                services: BTreeMap::new(),
                pagination_request_tx,
                pagination_failure_tx,
                tasks: SubscriptionTasks::new(vec![
                    pending_task(item_settled_tx),
                    pending_task(pagination_settled_tx),
                    pending_task(update_settled_tx),
                ]),
            },
            [item_settled_rx, pagination_settled_rx, update_settled_rx],
        )
    }

    async fn assert_tasks_settled(tasks: [oneshot::Receiver<()>; 3]) {
        for settled in tasks {
            crate::executor::timeout(Duration::from_millis(100), settled)
                .await
                .expect("every owned subscription task must settle");
        }
    }

    #[tokio::test]
    async fn active_subscription_shutdown_settles_every_owned_task() {
        let (active, tasks) = pending_subscription();
        active.shutdown().await;
        assert_tasks_settled(tasks).await;
    }

    #[tokio::test]
    async fn active_subscription_drop_aborts_every_owned_task() {
        let (active, tasks) = pending_subscription();
        drop(active);
        assert_tasks_settled(tasks).await;
    }

    fn test_timeline_item(event_id: &str) -> TimelineItem {
        TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: None,
            sender_label: None,
            sender_avatar: None,
            body: Some("old root".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            unable_to_decrypt: None,
            actions: TimelineMessageActions::default(),
            send_state: None,
        }
    }

    #[test]
    fn sdk_aggregate_adapter_preserves_exact_count_and_latest_fields() {
        let aggregate = ThreadRelationAggregate {
            latest_event: Some(ThreadListItemEvent {
                event_id: event_id!("$reply:example.invalid").to_owned(),
                timestamp: MilliSecondsSinceUnixEpoch(matrix_sdk::ruma::UInt::new_saturating(42)),
                sender: user_id!("@sender:example.invalid").to_owned(),
                is_own: false,
                sender_profile: TimelineDetails::Ready(Profile {
                    display_name: Some("Sender".to_owned()),
                    ..Profile::default()
                }),
                content: None,
            }),
            num_replies: u32::MAX,
        };
        assert_eq!(
            authoritative_thread_aggregate_from_sdk(&aggregate),
            super::AuthoritativeThreadAggregate {
                reply_count: u32::MAX,
                latest_event_id: Some("$reply:example.invalid".to_owned()),
                latest_sender: Some("@sender:example.invalid".to_owned()),
                latest_sender_label: Some("Sender".to_owned()),
                latest_body_preview: None,
                latest_timestamp_ms: Some(42),
            }
        );
    }

    #[test]
    fn thread_root_projection_service_emits_one_bounded_fetch_and_never_retries_terminal_failure() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: Some("@user-b:example.invalid".to_owned()),
            activity_sender_label: Some("User B".to_owned()),
            activity_body_preview: Some("Latest preview".to_owned()),
        };

        assert_eq!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(activity.clone())
        );
        assert_eq!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::Existing(
                service
                    .attempts
                    .get(&(activity.room_id.clone(), activity.root_event_id.clone()))
                    .expect("pending record")
                    .clone()
            )
        );

        service.mark_failed(&activity, OperationFailureKind::NotFound);
        assert!(
            !service.has_pending_attempt(&activity),
            "failed hydration is terminal and must not be retried"
        );
        assert_eq!(
            service.observe(activity),
            ThreadRootProjectionDecision::Existing(
                service
                    .attempts
                    .get(&(
                        "!room:example.invalid".to_owned(),
                        "$old-root:example.invalid".to_owned()
                    ))
                    .expect("failed record")
                    .clone()
            ),
            "a failed root projection is terminal and must not loop"
        );
    }

    #[test]
    fn aggregate_refresh_is_pending_for_dto_but_not_hydration_dedupe() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let refresh = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::InitialHydration,
                true,
                false,
            )
            .expect("aggregate refresh");
        service
            .mark_ready(&activity, test_timeline_item(&activity.root_event_id))
            .expect("hydration terminal");

        let record = service
            .attempts
            .get(&(activity.room_id.clone(), activity.root_event_id.clone()))
            .expect("retained aggregate refresh");
        assert!(
            record.is_pending(),
            "DTO state stays pending during aggregation"
        );
        assert_eq!(record.pending_refresh(), Some(refresh));
        assert!(
            !service.has_pending_attempt(&activity),
            "an aggregate worker must not be mistaken for a hydration attempt"
        );
    }

    #[test]
    fn active_failed_root_survives_recreated_actor_but_is_eligible_after_active_window_cleanup() {
        let shared = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: Some("@user-b:example.invalid".to_owned()),
            activity_sender_label: Some("User B".to_owned()),
            activity_body_preview: Some("Latest preview".to_owned()),
        };

        // First Room actor starts and fails the sole bounded attempt.
        {
            let mut service = shared.lock().expect("test service lock");
            assert!(matches!(
                service.observe(activity.clone()),
                ThreadRootProjectionDecision::StartFetch(_)
            ));
            service.mark_failed(&activity, OperationFailureKind::NotFound);
            service.reconcile_room(
                &activity.room_id,
                &HashSet::from([activity.root_event_id.clone()]),
            );
        }

        // SyncStarted replaces the Room actor, but it must consult the same
        // Room-scoped service and emit the retained terminal record instead of
        // issuing a second load_or_fetch_event.
        {
            let mut replacement_actor_service = shared.lock().expect("test service lock");
            let decision = replacement_actor_service.observe(activity.clone());
            assert!(matches!(
                decision,
                ThreadRootProjectionDecision::Existing(record)
                    if record.failure_kind() == Some(OperationFailureKind::NotFound)
            ));
        }

        // Once the canonical reply window no longer contains this root, the
        // terminal state is evicted. A later observation is a new bounded
        // attempt rather than an automatic retry of an active failed reply.
        {
            let mut service = shared.lock().expect("test service lock");
            service.reconcile_room(&activity.room_id, &HashSet::new());
            assert!(matches!(
                service.observe(activity),
                ThreadRootProjectionDecision::StartFetch(_)
            ));
        }
    }

    #[test]
    fn active_failed_root_updates_to_newest_reply_without_starting_a_second_fetch() {
        let mut service = ThreadRootProjectionService::default();
        let first_activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$first-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: Some("@user-a:example.invalid".to_owned()),
            activity_sender_label: Some("User A".to_owned()),
            activity_body_preview: Some("First preview".to_owned()),
        };
        assert!(matches!(
            service.observe(first_activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        service.reconcile_room(
            &first_activity.room_id,
            &HashSet::from([first_activity.root_event_id.clone()]),
        );
        service.mark_failed(&first_activity, OperationFailureKind::NotFound);

        let newest_activity = ThreadRootProjectionActivity {
            activity_event_id: "$newest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_200_000),
            activity_sender: Some("@user-b:example.invalid".to_owned()),
            activity_sender_label: Some("User B".to_owned()),
            activity_body_preview: Some("Newest preview".to_owned()),
            ..first_activity
        };

        assert!(matches!(
            service.observe(newest_activity),
            ThreadRootProjectionDecision::ActivityUpdated(record)
                if record.failure_kind() == Some(OperationFailureKind::NotFound)
                    && record.activity.activity_event_id == "$newest-reply:example.invalid"
        ));
    }

    #[test]
    fn same_reply_identity_edit_advances_activity_revision_boundary() {
        let existing = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: Some("@sender:example.invalid".to_owned()),
            activity_sender_label: Some("Sender".to_owned()),
            activity_body_preview: Some("old".to_owned()),
        };
        let edited = ThreadRootProjectionActivity {
            activity_body_preview: Some("edited".to_owned()),
            ..existing.clone()
        };

        assert!(
            super::activity_is_newer(&edited, &existing),
            "same-ID/same-timestamp effective edits must advance activity fencing"
        );
    }

    #[test]
    fn newer_live_activity_floors_a_lagging_sdk_aggregate_without_double_counting() {
        let mut service = ThreadRootProjectionService::default();
        let activity_a = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply-a:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: Some("@a:example.invalid".to_owned()),
            activity_sender_label: Some("A".to_owned()),
            activity_body_preview: Some("A".to_owned()),
        };
        assert!(matches!(
            service.observe(activity_a.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let refresh_a = service
            .schedule_aggregate_refresh(
                &activity_a,
                AggregateRefreshCause::InitialHydration,
                true,
                false,
            )
            .expect("initial aggregate refresh");
        assert!(matches!(
            service.complete_refresh(
                &refresh_a,
                Ok(ThreadRootProjectionRefreshResult::Aggregate(
                    super::AuthoritativeThreadAggregate {
                        reply_count: 1,
                        latest_event_id: Some(activity_a.activity_event_id.clone()),
                        latest_sender: activity_a.activity_sender.clone(),
                        latest_sender_label: activity_a.activity_sender_label.clone(),
                        latest_body_preview: activity_a.activity_body_preview.clone(),
                        latest_timestamp_ms: activity_a.activity_timestamp_ms,
                    }
                )),
            ),
            super::ThreadRootProjectionCompletion::Updated(_)
        ));

        let activity_b = ThreadRootProjectionActivity {
            activity_event_id: "$reply-b:example.invalid".to_owned(),
            activity_timestamp_ms: Some(200),
            activity_sender: Some("@b:example.invalid".to_owned()),
            activity_sender_label: Some("B".to_owned()),
            activity_body_preview: Some("B".to_owned()),
            ..activity_a.clone()
        };
        assert!(matches!(
            service.observe(activity_b.clone()),
            ThreadRootProjectionDecision::ActivityUpdated(_)
        ));
        let refresh_b = service
            .schedule_aggregate_refresh(
                &activity_b,
                AggregateRefreshCause::SelectedActivity,
                true,
                false,
            )
            .expect("live aggregate refresh");
        let completion = service.complete_refresh(
            &refresh_b,
            Ok(ThreadRootProjectionRefreshResult::Aggregate(
                super::AuthoritativeThreadAggregate {
                    reply_count: 1,
                    latest_event_id: Some(activity_a.activity_event_id),
                    latest_sender: activity_a.activity_sender,
                    latest_sender_label: activity_a.activity_sender_label,
                    latest_body_preview: activity_a.activity_body_preview,
                    latest_timestamp_ms: activity_a.activity_timestamp_ms,
                },
            )),
        );
        assert!(matches!(
            completion,
            super::ThreadRootProjectionCompletion::Updated(record)
                if record.aggregate.reply_count == 2
                    && record.aggregate.latest_event_id.as_deref()
                        == Some("$reply-b:example.invalid")
        ));
    }

    #[test]
    fn aggregate_refresh_reconciles_count_two_to_one_to_zero() {
        let mut service = ThreadRootProjectionService::default();
        let activity_b = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply-b:example.invalid".to_owned(),
            activity_timestamp_ms: Some(200),
            activity_sender: Some("@b:example.invalid".to_owned()),
            activity_sender_label: Some("B".to_owned()),
            activity_body_preview: Some("B".to_owned()),
        };
        assert!(matches!(
            service.observe(activity_b.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let refresh = service
            .schedule_aggregate_refresh(
                &activity_b,
                AggregateRefreshCause::InitialHydration,
                true,
                false,
            )
            .expect("initial aggregate refresh");
        assert!(matches!(
            service.complete_refresh(
                &refresh,
                Ok(ThreadRootProjectionRefreshResult::Aggregate(
                    super::AuthoritativeThreadAggregate {
                        reply_count: 2,
                        latest_event_id: Some("$reply-b:example.invalid".to_owned()),
                        latest_sender: Some("@b:example.invalid".to_owned()),
                        latest_sender_label: Some("B".to_owned()),
                        latest_body_preview: Some("B".to_owned()),
                        latest_timestamp_ms: Some(200),
                    }
                )),
            ),
            super::ThreadRootProjectionCompletion::Updated(_)
        ));

        let activity_a = ThreadRootProjectionActivity {
            activity_event_id: "$reply-a:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: Some("@a:example.invalid".to_owned()),
            activity_sender_label: Some("A".to_owned()),
            activity_body_preview: Some("A".to_owned()),
            ..activity_b.clone()
        };
        assert!(matches!(
            service.observe(activity_a.clone()),
            ThreadRootProjectionDecision::ActivityUpdated(_)
        ));
        let refresh = service
            .schedule_aggregate_refresh(
                &activity_a,
                AggregateRefreshCause::SelectedActivity,
                true,
                false,
            )
            .expect("changed activity aggregate refresh");
        let completion = service.complete_refresh(
            &refresh,
            Ok(ThreadRootProjectionRefreshResult::Aggregate(
                super::AuthoritativeThreadAggregate {
                    reply_count: 1,
                    latest_event_id: Some("$reply-a:example.invalid".to_owned()),
                    latest_sender: Some("@a:example.invalid".to_owned()),
                    latest_sender_label: Some("A".to_owned()),
                    latest_body_preview: Some("A".to_owned()),
                    latest_timestamp_ms: Some(100),
                },
            )),
        );
        assert!(
            matches!(completion, super::ThreadRootProjectionCompletion::Updated(record)
            if record.aggregate.reply_count == 1
                && record.aggregate.latest_event_id.as_deref() == Some("$reply-a:example.invalid"))
        );

        service.reconcile_room_with_affected(
            &activity_a.room_id,
            &HashSet::new(),
            &HashSet::from([activity_a.root_event_id.clone()]),
        );
        let refresh = service
            .schedule_aggregate_refresh(&activity_a, AggregateRefreshCause::Removal, false, false)
            .expect("disappeared root aggregate refresh");
        assert!(matches!(
            service.complete_refresh(
                &refresh,
                Ok(ThreadRootProjectionRefreshResult::Aggregate(
                    super::AuthoritativeThreadAggregate::default()
                )),
            ),
            super::ThreadRootProjectionCompletion::Cleared(_)
        ));
        assert!(
            service
                .terminal_record(&activity_a.room_id, &activity_a.root_event_id)
                .is_none()
        );
    }

    #[test]
    fn hydrated_zero_count_for_inactive_root_clears_the_retained_record() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let refresh = service
            .schedule_aggregate_refresh(&activity, AggregateRefreshCause::Removal, false, false)
            .expect("inactive hydration refresh");

        assert!(matches!(
            service.complete_refresh(
                &refresh,
                Ok(ThreadRootProjectionRefreshResult::Hydrated {
                    item: test_timeline_item(&activity.root_event_id),
                    aggregate: super::AuthoritativeThreadAggregate::default(),
                }),
            ),
            super::ThreadRootProjectionCompletion::Cleared(cleared)
                if cleared == activity
        ));
        assert!(
            service
                .terminal_record(&activity.room_id, &activity.root_event_id)
                .is_none(),
            "zero-count inactive hydration must remove the record"
        );
    }

    #[test]
    fn aggregate_refresh_ignores_stale_completion_and_retires_exhausted_serials() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let first = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::CanonicalBatch,
                true,
                false,
            )
            .expect("first refresh");
        let second = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::CanonicalBatch,
                true,
                false,
            )
            .expect("newer refresh");
        assert!(matches!(
            service.complete_refresh(
                &first,
                Ok(ThreadRootProjectionRefreshResult::Aggregate(
                    super::AuthoritativeThreadAggregate {
                        reply_count: 9,
                        ..Default::default()
                    }
                )),
            ),
            super::ThreadRootProjectionCompletion::Ignored
        ));
        assert!(matches!(
            service.complete_refresh(
                &second,
                Err(OperationFailureKind::Network),
            ),
            super::ThreadRootProjectionCompletion::Updated(record)
                if record.failure_kind() == Some(OperationFailureKind::Network)
        ));

        let record = service
            .attempts
            .get_mut(&(activity.room_id.clone(), activity.root_event_id.clone()))
            .expect("record");
        record.activity_revision = u64::MAX;
        assert!(matches!(
            service.observe(ThreadRootProjectionActivity {
                activity_event_id: "$new-reply:example.invalid".to_owned(),
                ..activity.clone()
            }),
            ThreadRootProjectionDecision::Retired
        ));
        assert!(
            service
                .schedule_aggregate_refresh(
                    &activity,
                    AggregateRefreshCause::CanonicalBatch,
                    true,
                    true,
                )
                .is_none()
        );
    }

    #[test]
    fn disappeared_aggregate_error_clears_the_retained_record() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let initial = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::InitialHydration,
                true,
                false,
            )
            .expect("initial refresh");
        let _ = service.complete_refresh(
            &initial,
            Ok(ThreadRootProjectionRefreshResult::Aggregate(
                super::AuthoritativeThreadAggregate {
                    reply_count: 2,
                    ..Default::default()
                },
            )),
        );
        service.reconcile_room_with_affected(
            &activity.room_id,
            &HashSet::new(),
            &HashSet::from([activity.root_event_id.clone()]),
        );
        let disappeared = service
            .schedule_aggregate_refresh(&activity, AggregateRefreshCause::Removal, false, false)
            .expect("disappearance refresh");
        assert!(matches!(
            service.complete_refresh(&disappeared, Err(OperationFailureKind::Sdk)),
            super::ThreadRootProjectionCompletion::Cleared(_)
        ));
    }

    #[test]
    fn aggregate_refresh_has_production_manager_start_and_finish_callers() {
        let source = include_str!("timeline/manager.rs");
        assert!(source.contains("StartAggregateRefresh"));
        assert!(source.contains("AggregateRefreshFinished"));
        assert!(source.contains("handle_aggregate_refresh"));

        let thread_projection = include_str!("timeline/thread_projection.rs");
        let commit = thread_projection
            .split_once("async fn commit_prepared_thread_root_hydration_for_generation(")
            .and_then(|(_, source)| {
                source.split_once("fn thread_root_projection_action_from_record")
            })
            .map(|(source, _)| source)
            .expect("thread-root hydration commit source");
        assert!(commit.contains("schedule_aggregate_refresh"));
        assert!(commit.contains("StartAggregateRefresh"));
    }

    #[test]
    fn reconciliation_moves_ready_and_failed_records_to_the_remaining_older_reply_without_fetching()
    {
        for failure_kind in [None, Some(OperationFailureKind::NotFound)] {
            let mut service = ThreadRootProjectionService::default();
            let newer = ThreadRootProjectionActivity {
                room_id: "!room:example.invalid".to_owned(),
                root_event_id: "$old-root:example.invalid".to_owned(),
                activity_event_id: "$newer-reply:example.invalid".to_owned(),
                activity_timestamp_ms: Some(200),
                activity_sender: None,
                activity_sender_label: None,
                activity_body_preview: None,
            };
            let older = ThreadRootProjectionActivity {
                activity_event_id: "$older-reply:example.invalid".to_owned(),
                activity_timestamp_ms: Some(100),
                ..newer.clone()
            };
            assert!(matches!(
                service.observe(newer.clone()),
                ThreadRootProjectionDecision::StartFetch(_)
            ));
            service.reconcile_room_activities(
                &newer.room_id,
                &HashMap::from([(newer.root_event_id.clone(), newer.clone())]),
            );
            match failure_kind {
                Some(failure_kind) => {
                    service.mark_failed(&newer, failure_kind);
                }
                None => {
                    service.mark_ready(&newer, test_timeline_item(&newer.root_event_id));
                }
            }

            // The newest reply is no longer canonical. Reconciliation, rather
            // than observe(), is allowed to move the representative backward.
            service.reconcile_room_activities(
                &older.room_id,
                &HashMap::from([(older.root_event_id.clone(), older.clone())]),
            );
            assert!(matches!(
                service.observe(older.clone()),
                ThreadRootProjectionDecision::Existing(record)
                    if record.activity == older
                        && record.failure_kind() == failure_kind
                        && (failure_kind.is_some() || record.item().is_some())
            ));
        }
    }

    #[test]
    fn clearing_an_unsubscribed_room_allows_a_later_room_actor_to_start_a_fresh_attempt() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        assert_eq!(service.clear_room(&activity.room_id).len(), 1);
        assert!(matches!(
            service.observe(activity),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
    }

    #[test]
    fn inactive_pending_completion_returns_terminal_snapshot_for_state_cleanup_then_evicts_core_record()
     {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        service.reconcile_room(&activity.room_id, &HashSet::new());

        let completed = service
            .mark_failed(&activity, OperationFailureKind::NotFound)
            .expect(
                "the terminal result must reach state/frontend cleanup even after activity leaves",
            );
        assert_eq!(
            completed.failure_kind(),
            Some(OperationFailureKind::NotFound)
        );
        assert!(
            !service
                .active_root_event_ids
                .contains_key(&activity.room_id),
            "an inactive room with no pending records must not leave a session-long empty marker"
        );
        assert!(matches!(
            service.observe(activity),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
    }

    #[test]
    fn ready_snapshot_remains_reemittable_after_temporary_canonical_root_overlap() {
        let mut service = ThreadRootProjectionService::default();
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$old-root:example.invalid".to_owned(),
            activity_event_id: "$latest-reply:example.invalid".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        service.reconcile_room(
            &activity.room_id,
            &HashSet::from([activity.root_event_id.clone()]),
        );
        let item = TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: activity.root_event_id.clone(),
            },
            sender: None,
            sender_label: None,
            sender_avatar: None,
            body: Some("old root".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            unable_to_decrypt: None,
            actions: TimelineMessageActions::default(),
            send_state: None,
        };
        service
            .mark_ready(&activity, item)
            .expect("the reply remains active even while its root is canonical");

        assert!(matches!(
            service.observe(activity),
            ThreadRootProjectionDecision::Existing(record) if record.item().is_some()
        ));
    }

    #[test]
    fn thread_root_projection_source_never_uses_room_pagination_or_anchor_materialization() {
        let source = include_str!("threads_list.rs");
        let projection_section = source
            .split("pub struct ThreadRootProjectionService")
            .nth(1)
            .expect("thread-root projection service must be present")
            .split("#[cfg(test)]")
            .next()
            .expect("projection production section");

        assert!(
            !projection_section.contains("paginate_backwards")
                && !projection_section.contains("PaginateBackward")
                && !projection_section.contains("RestoreTimelineAnchor"),
            "root hydration must stay bounded to load_or_fetch_event; it must not page or materialize anchors"
        );
    }

    #[test]
    fn open_subscription_loads_initial_page_before_emitting_opened() {
        let source = include_str!("threads_list.rs");
        let open_subscription = source
            .split("async fn open_subscription")
            .nth(1)
            .expect("open_subscription body")
            .split("async fn emit_opened")
            .next()
            .expect("open_subscription section");
        let paginate_index = open_subscription
            .find("service.paginate().await")
            .expect("open_subscription must load the first thread page");
        let emit_index = open_subscription
            .find("self.emit_opened")
            .expect("open_subscription must emit opened");

        assert!(
            paginate_index < emit_index,
            "ThreadListService::new() starts empty; paginate before emitting Opened"
        );
    }

    #[test]
    fn paginate_updates_are_correlated_to_paginate_request_id() {
        let source = include_str!("threads_list.rs");
        let active_paginate = source
            .split("impl ActiveSubscription")
            .nth(1)
            .expect("ActiveSubscription impl")
            .split("async fn paginate(&self, request_id: RequestId)")
            .nth(1)
            .expect("ActiveSubscription::paginate body")
            .split("fn project_item")
            .next()
            .expect("ActiveSubscription section");
        assert!(
            active_paginate.contains("send(request_id)"),
            "pagination must hand the fresh paginate request id to the update task"
        );
        assert!(
            !active_paginate.contains("let _ = request_id"),
            "pagination must not discard the fresh request id"
        );

        let pagination_updates = source
            .split("Some((_, _state)) = pagination_rx.recv()")
            .nth(1)
            .expect("pagination update branch")
            .split("else => break")
            .next()
            .expect("pagination update section");
        assert!(
            pagination_updates.contains("current_request_id.sequence"),
            "pagination state actions must use the current paginate request id"
        );
        assert!(
            !pagination_updates.contains("request_id: request_id.sequence"),
            "pagination state actions must not keep using the open request id"
        );
    }

    #[test]
    fn thread_list_relays_are_reliable_and_paginate_errors_fail() {
        let source = include_str!("threads_list.rs");
        let production_source = source
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .expect("production source should precede tests");
        let open_subscription = production_source
            .split("async fn open_subscription")
            .nth(1)
            .expect("open_subscription body")
            .split("async fn emit_opened")
            .next()
            .expect("open_subscription section");
        let active_paginate = production_source
            .split("impl ActiveSubscription")
            .nth(1)
            .expect("ActiveSubscription impl")
            .split("async fn paginate(&self, request_id: RequestId)")
            .nth(1)
            .expect("ActiveSubscription::paginate body")
            .split("fn project_item")
            .next()
            .expect("ActiveSubscription section");

        assert!(
            !open_subscription.contains("try_send"),
            "thread-list item/pagination relays must not drop terminal updates"
        );
        assert!(
            open_subscription.contains("items_tx.send(room_id.clone()).await"),
            "item relay should await reliable delivery to the update task"
        );
        assert!(
            open_subscription.contains("pagination_tx.send((room_id.clone(), state)).await"),
            "pagination relay should await terminal state delivery to the update task"
        );
        assert!(
            active_paginate.contains("classify_thread_list_error(&error)"),
            "paginate errors must be classified instead of reported as success through Idle"
        );
        assert!(
            active_paginate.contains("pagination_failure_tx"),
            "paginate errors must reach the update task through a reliable failure relay"
        );
        assert!(
            open_subscription.contains("AppAction::ThreadsListFailed"),
            "paginate errors must project an explicit failed settle"
        );
        assert!(
            open_subscription.contains("failed_pagination_request_id"),
            "the Idle state emitted after an SDK pagination error must not overwrite Failed"
        );
        assert!(
            open_subscription
                .contains("self.emit_failed(&scope, request_id, OperationFailureKind::Invalid)")
                && open_subscription.contains(
                    "self.emit_failed(&scope, request_id, OperationFailureKind::NotFound)"
                ),
            "open failures should preserve parse/not-found failure kinds"
        );
    }
}
