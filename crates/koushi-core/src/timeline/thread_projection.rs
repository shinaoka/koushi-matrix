use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, AvatarImage, AvatarThumbnailState, OperationFailureKind,
    ThreadRootProjectionActivity as ThreadRootProjectionActivityState,
};

use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::events::sticker::StickerEventContent;
use matrix_sdk_ui::timeline::{
    EventItemOrigin, EventTimelineItem, TimelineItem as SdkTimelineItem,
    resolve_thread_relation_aggregate,
};
use tokio::sync::{broadcast, mpsc};

use crate::causal_projection::CausalProjectionId;
use crate::event::{
    CoreEvent, ReactionGroup, ReactionSender, ThreadRootProjectionDto,
    ThreadRootProjectionSourceDto, ThreadRootProjectionStateDto, ThreadSummaryDto, TimelineEvent,
    TimelineItem, TimelineItemId, TimelineUnableToDecrypt, TimelineUnableToDecryptReason,
    message_actions_for_timeline_item,
};
use crate::executor;
use crate::ids::{TimelineKey, TimelineKind};
use crate::threads_list::{
    AggregateRefresh, AggregateRefreshCause, ThreadRootProjectionActivity,
    ThreadRootProjectionDecision, ThreadRootProjectionRecord, ThreadRootProjectionRefreshResult,
    ThreadRootProjectionService, activity_is_newer, authoritative_thread_aggregate_from_sdk,
    classify_thread_list_error,
};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{ThreadSummaryProjectionWake, TimelineActor};
use super::item_projection::{
    MessageProjection, eligible_activity_preview, is_attention_eligible_event,
    link_ranges_for_message_projection, message_projection_from_msgtype,
    non_user_content_projection, sticker_projection_from_body, timeline_content_is_renderable,
    timeline_item_event_id,
};
use super::manager::{TimelineManagerActor, TimelineMessage};
use super::navigation::{TimelineActorGenerationGate, emit_timeline_events_with_lease};
use super::outbound_send::{
    matching_remote_thread_reply_event_id, matching_thread_reply_event_id, thread_attention_action,
};
// END GENERATED SIBLING IMPORTS

/// A bounded Room replay may surface summary-only roots that the SDK omitted
/// from its item stream. Keep this much smaller than the base window so a
/// historical root-heavy room cannot multiply initial render work.

const ROOM_REPLAY_KNOWN_THREAD_ROOT_PROJECTIONS_MAX: usize = 32;

/// `epoch` crosses the JSON/JavaScript IPC boundary as a number. It must stay
/// within JavaScript's exact integer range so a source-scoped Clear can never
/// be rounded into another replay owner's epoch.
pub(super) const JAVASCRIPT_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;

/// Manager-owned tasks for bounded root hydration and aggregate refresh.
/// Removing a task before a queued completion is handled makes the completion
/// stale by construction. The optional revision distinguishes an aggregate
/// worker from a root-hydration worker and fences an old aggregate completion
/// from removing a newer worker for the same root.
#[derive(Default)]
pub(super) struct ThreadRootProjectionFetchRegistry {
    tasks: HashMap<(String, String), (u64, Option<u64>, executor::JoinHandle<()>)>,
}

impl ThreadRootProjectionFetchRegistry {
    fn contains_hydration(
        &self,
        room_id: &str,
        root_event_id: &str,
        actor_generation: u64,
    ) -> bool {
        self.tasks
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .is_some_and(|(generation, revision, _)| {
                *generation == actor_generation && revision.is_none()
            })
    }

    fn contains_aggregate(
        &self,
        room_id: &str,
        root_event_id: &str,
        actor_generation: u64,
        summary_revision: u64,
    ) -> bool {
        self.tasks
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
            .is_some_and(|(generation, revision, _)| {
                *generation == actor_generation && *revision == Some(summary_revision)
            })
    }

    fn insert(
        &mut self,
        room_id: String,
        root_event_id: String,
        actor_generation: u64,
        summary_revision: Option<u64>,
        task: executor::JoinHandle<()>,
    ) {
        if let Some((_, _, previous)) = self.tasks.insert(
            (room_id, root_event_id),
            (actor_generation, summary_revision, task),
        ) {
            previous.abort();
        }
    }

    /// Returns false when unsubscribe, replacement, or a newer refresh already
    /// cancelled this worker; callers must ignore its late terminal message.
    fn take_completion(
        &mut self,
        room_id: &str,
        root_event_id: &str,
        actor_generation: u64,
        summary_revision: Option<u64>,
    ) -> bool {
        let key = (room_id.to_owned(), root_event_id.to_owned());
        if self
            .tasks
            .get(&key)
            .is_some_and(|(generation, revision, _)| {
                *generation == actor_generation && *revision == summary_revision
            })
        {
            self.tasks.remove(&key);
            true
        } else {
            false
        }
    }

    fn abort_room(&mut self, room_id: &str) -> usize {
        let keys = self
            .tasks
            .keys()
            .filter(|(entry_room_id, _)| entry_room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();
        let count = keys.len();
        for key in keys {
            if let Some((_, _, task)) = self.tasks.remove(&key) {
                task.abort();
            }
        }
        count
    }

    pub(super) fn abort_all(&mut self) {
        for (_, (_, _, task)) in self.tasks.drain() {
            task.abort();
        }
    }
}

/// Lifecycle registry for ready root snapshots copied from an actor's own
/// navigation cache during a bounded replay. This is separate from the
/// fetch-backed projection service: no SDK fetch was started for these roots,
/// but unsubscribe and shutdown still must emit a matching frontend clear.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReplayKnownThreadRootProjection {
    pub(super) root_event_id: String,
    pub(super) activity_event_id: String,
    pub(super) activity_timestamp_ms: Option<u64>,
    /// The full renderable Ready payload. Activity identity alone is not a
    /// revision: edits, redactions, reactions, and action affordances can
    /// change while the latest thread reply remains the same.
    pub(super) item: TimelineItem,
    pub(super) source: ThreadRootProjectionSourceDto,
}

#[derive(Default)]
pub(super) struct ReplayKnownThreadRootProjectionRegistry {
    pub(super) entries: HashMap<TimelineKey, HashMap<String, ReplayKnownThreadRootProjection>>,
    /// Hydration terminal results that arrived while a replay-known Ready
    /// owned the root. The marker is consumed when that owner clears.
    pub(super) suppressed_hydration_terminals: HashMap<TimelineKey, HashSet<String>>,
    /// Hydration terminal results that were actually broadcast while no replay
    /// owner existed. A later replay Ready can overwrite that source in the
    /// desktop store, so its scoped Clear must reassert this terminal. Merely
    /// retaining a terminal in the service is not sufficient: it may never
    /// have been visible to the store.
    pub(super) emitted_hydration_terminals: HashMap<TimelineKey, HashSet<String>>,
    pub(super) next_epoch: u64,
}

#[derive(Default)]
pub(super) struct ReplayKnownThreadRootProjectionUpdate {
    pub(super) ready: Vec<ThreadRootProjectionDto>,
    pub(super) stale: Vec<ReplayKnownThreadRootProjection>,
}

impl TimelineManagerActor {
    pub(super) async fn handle_thread_root_projection_fetch_start(
        &mut self,
        key: TimelineKey,
        actor_generation: u64,
        own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
        activities: Vec<ThreadRootProjectionActivity>,
    ) {
        let Some(_lease) = self
            .timeline_actor_generations
            .try_acquire(&key, actor_generation)
        else {
            return;
        };
        if !matches!(key.kind, TimelineKind::Room { .. }) || !self.timelines.contains_key(&key) {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        for activity in activities {
            if self.thread_root_projection_fetches.contains_hydration(
                &activity.room_id,
                &activity.root_event_id,
                actor_generation,
            ) {
                continue;
            }
            let should_start = self
                .thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned")
                .has_pending_attempt(&activity);
            if !should_start {
                continue;
            }
            let task = spawn_thread_root_projection_fetch(
                session.clone(),
                key.clone(),
                actor_generation,
                own_user_id.clone(),
                self.msg_tx.clone(),
                activity.clone(),
            );
            self.thread_root_projection_fetches.insert(
                activity.room_id,
                activity.root_event_id,
                actor_generation,
                None,
                task,
            );
        }
    }
    pub(super) async fn handle_thread_root_projection_fetch_finished(
        &mut self,
        key: TimelineKey,
        actor_generation: u64,
        activity: ThreadRootProjectionActivity,
        result: Result<TimelineItem, OperationFailureKind>,
    ) {
        if !self.thread_root_projection_fetches.take_completion(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
            None,
        ) || !self.timelines.contains_key(&key)
        {
            return;
        }
        let Ok(action_permit) = self.action_tx.clone().reserve_owned().await else {
            return;
        };
        let Some(lease) = self
            .timeline_actor_generations
            .try_acquire(&key, actor_generation)
        else {
            return;
        };
        let mut service = self
            .thread_root_projection_service
            .lock()
            .expect("thread-root projection service lock must not be poisoned");
        let record = match result {
            Ok(item) => service.mark_ready(&activity, item),
            Err(failure_kind) => service.mark_failed(&activity, failure_kind),
        };
        let Some(record) = record else {
            return;
        };
        let pending_refresh = record.pending_refresh();
        action_permit.send(vec![thread_root_projection_action_from_record(&record)]);
        drop(service);
        // A bounded replay may have acquired display ownership while this
        // manager-owned cache/network lookup was in flight. The shared
        // registry mutex covers both this decision and the synchronous
        // broadcast: actor replay publication uses the same boundary, so no
        // replay Ready can land between a no-owner check and hydration's
        // terminal event. The terminal remains in the service/reducer state
        // either way and is handed back when replay ownership later ends.
        let _ = emit_hydration_terminal_unless_replay_owned(
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &key,
            thread_root_projection_dto_from_record(&record),
        );
        drop(lease);
        if let Some(refresh) = pending_refresh {
            self.start_aggregate_worker(
                &key,
                actor_generation,
                self.session
                    .as_ref()
                    .and_then(|session| session.client().user_id().map(ToOwned::to_owned)),
                refresh,
            );
        }
    }

    pub(super) async fn handle_aggregate_refresh_start(
        &mut self,
        key: TimelineKey,
        actor_generation: u64,
        own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
        refreshes: Vec<AggregateRefresh>,
    ) {
        let Some(_lease) = self
            .timeline_actor_generations
            .try_acquire(&key, actor_generation)
        else {
            return;
        };
        if !matches!(key.kind, TimelineKind::Room { .. }) || !self.timelines.contains_key(&key) {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        for refresh in refreshes {
            if refresh.hydrate_root {
                if self.thread_root_projection_fetches.contains_aggregate(
                    &refresh.activity.room_id,
                    &refresh.activity.root_event_id,
                    actor_generation,
                    refresh.summary_revision,
                ) {
                    continue;
                }
                if self.thread_root_projection_fetches.contains_hydration(
                    &refresh.activity.room_id,
                    &refresh.activity.root_event_id,
                    actor_generation,
                ) {
                    continue;
                }
                let should_start = self
                    .thread_root_projection_service
                    .lock()
                    .expect("thread-root projection service lock must not be poisoned")
                    .has_pending_attempt(&refresh.activity);
                if !should_start {
                    continue;
                }
                let task = spawn_thread_root_projection_fetch(
                    session.clone(),
                    key.clone(),
                    actor_generation,
                    own_user_id.clone(),
                    self.msg_tx.clone(),
                    refresh.activity.clone(),
                );
                self.thread_root_projection_fetches.insert(
                    refresh.activity.room_id.clone(),
                    refresh.activity.root_event_id.clone(),
                    actor_generation,
                    None,
                    task,
                );
            } else {
                self.start_aggregate_worker(&key, actor_generation, own_user_id.clone(), refresh);
            }
        }
    }

    pub(super) fn handle_thread_summary_activity_observed(
        &mut self,
        source_key: TimelineKey,
        actor_generation: u64,
        observation: ThreadSummaryActivityObservation,
    ) {
        let TimelineKind::Thread {
            room_id,
            root_event_id,
        } = &source_key.kind
        else {
            return;
        };
        if self
            .timeline_actor_generations
            .current_generation(&source_key)
            != Some(actor_generation)
            || !self.timelines.contains_key(&source_key)
        {
            return;
        }
        let room_key = TimelineKey::room(source_key.account_key.clone(), room_id.clone());
        let Some(room_actor_generation) = self
            .timeline_actor_generations
            .current_generation(&room_key)
        else {
            return;
        };
        if !self.timelines.contains_key(&room_key) {
            return;
        }
        let mut refreshes = Vec::new();
        {
            let mut service = self
                .thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned");
            match observation {
                ThreadSummaryActivityObservation::Activity(activity)
                    if activity.root_event_id == *root_event_id =>
                {
                    let decision = service.observe_live_activity(activity.clone());
                    let should_refresh = matches!(
                        decision,
                        ThreadRootProjectionDecision::StartFetch(_)
                            | ThreadRootProjectionDecision::ActivityUpdated(_)
                    );
                    if should_refresh {
                        let activity_active = service.activity_active(room_id, root_event_id);
                        let canonical_root_active =
                            service.canonical_root_active(room_id, root_event_id);
                        if let Some(refresh) = service
                            .schedule_aggregate_refresh_with_canonical_root(
                                &activity,
                                AggregateRefreshCause::SelectedActivity,
                                activity_active,
                                canonical_root_active,
                                false,
                            )
                        {
                            refreshes.push(refresh);
                        }
                    }
                }
                ThreadSummaryActivityObservation::Invalidated {
                    root_event_id: invalidated_root_event_id,
                    activity_event_id,
                } if invalidated_root_event_id == *root_event_id => {
                    let invalidated = service.invalidate_live_activity(
                        room_id,
                        root_event_id,
                        &activity_event_id,
                    );
                    if invalidated
                        && let Some(activity) = service.activity_for_root(room_id, root_event_id)
                    {
                        let activity_active = service.activity_active(room_id, root_event_id);
                        let canonical_root_active =
                            service.canonical_root_active(room_id, root_event_id);
                        if let Some(refresh) = service
                            .schedule_aggregate_refresh_with_canonical_root(
                                &activity,
                                AggregateRefreshCause::Removal,
                                activity_active,
                                canonical_root_active,
                                false,
                            )
                        {
                            refreshes.push(refresh);
                        }
                    }
                }
                _ => return,
            }
        }
        let own_user_id = self
            .session
            .as_ref()
            .and_then(|session| session.client().user_id().map(ToOwned::to_owned));
        // Application travels through the Room actor's watch sender, never
        // through `TimelineActorHandle::send`.
        for refresh in refreshes {
            self.start_aggregate_worker(
                &room_key,
                room_actor_generation,
                own_user_id.clone(),
                refresh,
            );
        }
    }

    fn start_aggregate_worker(
        &mut self,
        key: &TimelineKey,
        actor_generation: u64,
        own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
        refresh: AggregateRefresh,
    ) {
        if !self.timelines.contains_key(key)
            || self.thread_root_projection_fetches.contains_hydration(
                &refresh.activity.room_id,
                &refresh.activity.root_event_id,
                actor_generation,
            )
            || self.thread_root_projection_fetches.contains_aggregate(
                &refresh.activity.room_id,
                &refresh.activity.root_event_id,
                actor_generation,
                refresh.summary_revision,
            )
        {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let task = spawn_aggregate_refresh(
            session,
            key.clone(),
            actor_generation,
            own_user_id,
            self.msg_tx.clone(),
            refresh.clone(),
        );
        self.thread_root_projection_fetches.insert(
            refresh.activity.room_id.clone(),
            refresh.activity.root_event_id.clone(),
            actor_generation,
            Some(refresh.summary_revision),
            task,
        );
    }

    pub(super) async fn handle_aggregate_refresh_finished(
        &mut self,
        key: TimelineKey,
        actor_generation: u64,
        refresh: AggregateRefresh,
        result: Result<ThreadRootProjectionRefreshResult, OperationFailureKind>,
    ) {
        if !self.thread_root_projection_fetches.take_completion(
            &refresh.activity.room_id,
            &refresh.activity.root_event_id,
            actor_generation,
            Some(refresh.summary_revision),
        ) || !self.timelines.contains_key(&key)
        {
            return;
        }
        let Some(lease) = self
            .timeline_actor_generations
            .try_acquire(&key, actor_generation)
        else {
            return;
        };
        let completion = self
            .thread_root_projection_service
            .lock()
            .expect("thread-root projection service lock must not be poisoned")
            .complete_refresh(&refresh, result);
        match completion {
            crate::threads_list::ThreadRootProjectionCompletion::Updated(record) => {
                let wake = ThreadSummaryProjectionWake {
                    root_event_id: record.activity.root_event_id.clone(),
                    activity_revision: record.activity_revision,
                    summary_revision: record.summary_revision,
                };
                for (timeline_key, actor) in &self.timelines {
                    let presents_root = match &timeline_key.kind {
                        TimelineKind::Room { .. } => true,
                        TimelineKind::Thread { root_event_id, .. } => {
                            root_event_id == &wake.root_event_id
                        }
                        TimelineKind::Focused { event_id, .. } => event_id == &wake.root_event_id,
                    };
                    if presents_root
                        && timeline_key.account_key == key.account_key
                        && timeline_key.room_id() == key.room_id()
                    {
                        actor.thread_summary_projection().publish(wake.clone());
                    }
                }
                if !refresh.canonical_root_active {
                    let Ok(action_permit) = self.action_tx.clone().reserve_owned().await else {
                        drop(lease);
                        return;
                    };
                    action_permit.send(vec![thread_root_projection_action_from_record(&record)]);
                    let _ = emit_hydration_terminal_unless_replay_owned(
                        &self.event_tx,
                        &self.replay_known_thread_root_projections,
                        &key,
                        thread_root_projection_dto_from_record(&record),
                    );
                }
            }
            crate::threads_list::ThreadRootProjectionCompletion::Cleared(activity) => {
                let Ok(action_permit) = self.action_tx.clone().reserve_owned().await else {
                    drop(lease);
                    return;
                };
                action_permit.send(vec![AppAction::ThreadRootProjectionCleared {
                    room_id: activity.room_id.clone(),
                    root_event_id: activity.root_event_id.clone(),
                }]);
                let _ = emit_hydration_terminal_unless_replay_owned(
                    &self.event_tx,
                    &self.replay_known_thread_root_projections,
                    &key,
                    ThreadRootProjectionDto {
                        root_event_id: activity.root_event_id,
                        activity_event_id: activity.activity_event_id,
                        activity_timestamp_ms: activity.activity_timestamp_ms,
                        retain_without_reply: false,
                        source: ThreadRootProjectionSourceDto::Hydration,
                        state: ThreadRootProjectionStateDto::Cleared,
                    },
                );
            }
            crate::threads_list::ThreadRootProjectionCompletion::Ignored => {}
        }
        drop(lease);
    }

    pub(super) async fn clear_thread_root_projections_for_room(&mut self, key: &TimelineKey) {
        if !matches!(key.kind, TimelineKind::Room { .. }) {
            return;
        }
        // Stop an old actor from acquiring a replay-known lease, then wait
        // only for its already-synchronous registry/Core emission section.
        // This releases the gate mutex before awaiting the watch notification.
        self.timeline_actor_generations
            .invalidate_and_quiesce(key)
            .await;
        let room_id = key.room_id();
        self.thread_root_projection_fetches.abort_room(room_id);
        let records = self
            .thread_root_projection_service
            .lock()
            .expect("thread-root projection service lock must not be poisoned")
            .clear_room(room_id);
        let replay_known = self
            .replay_known_thread_root_projections
            .lock()
            .expect("replay-known root registry lock must not be poisoned")
            .clear(key);
        let _ = self
            .emit_action_reliable(AppAction::ThreadRootProjectionsCleared {
                room_id: room_id.to_owned(),
            })
            .await;
        for record in records {
            self.emit(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                key: key.clone(),
                projection: ThreadRootProjectionDto {
                    root_event_id: record.activity.root_event_id,
                    activity_event_id: record.activity.activity_event_id,
                    activity_timestamp_ms: record.activity.activity_timestamp_ms,
                    retain_without_reply: false,
                    source: ThreadRootProjectionSourceDto::Hydration,
                    state: ThreadRootProjectionStateDto::Cleared,
                },
            }));
        }
        for projection in replay_known {
            self.emit(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                key: key.clone(),
                projection: replay_known_clear_projection(projection),
            }));
        }
    }
}

/// Starts the only allowed old-root hydration operation. It performs one
/// cache-first `load_or_fetch_event` call and reports a typed terminal outcome
/// back to the owning manager. It intentionally has no access to the SDK
/// `Timeline`, so backward pagination and anchor materialization are not
/// possible from this path.
fn spawn_thread_root_projection_fetch(
    session: Arc<MatrixClientSession>,
    key: TimelineKey,
    actor_generation: u64,
    own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
    manager_tx: mpsc::Sender<TimelineMessage>,
    activity: ThreadRootProjectionActivity,
) -> executor::JoinHandle<()> {
    executor::spawn(async move {
        let result =
            load_thread_root_projection_item(&session, &key, own_user_id.as_deref(), &activity)
                .await;
        let _ = manager_tx
            .send(TimelineMessage::ThreadRootProjectionFetchFinished {
                key,
                actor_generation,
                activity,
                result,
            })
            .await;
    })
}

fn spawn_aggregate_refresh(
    session: Arc<MatrixClientSession>,
    key: TimelineKey,
    actor_generation: u64,
    own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
    manager_tx: mpsc::Sender<TimelineMessage>,
    refresh: AggregateRefresh,
) -> executor::JoinHandle<()> {
    executor::spawn(async move {
        let result =
            resolve_aggregate_refresh(&session, &key, own_user_id.as_deref(), &refresh).await;
        let _ = manager_tx
            .send(TimelineMessage::AggregateRefreshFinished {
                key,
                actor_generation,
                refresh,
                result,
            })
            .await;
    })
}

async fn resolve_aggregate_refresh(
    session: &MatrixClientSession,
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    refresh: &AggregateRefresh,
) -> Result<ThreadRootProjectionRefreshResult, OperationFailureKind> {
    let room_id = matrix_sdk::ruma::RoomId::parse(refresh.activity.room_id.as_str())
        .map_err(|_| OperationFailureKind::Invalid)?;
    let root_event_id = matrix_sdk::ruma::EventId::parse(refresh.activity.root_event_id.as_str())
        .map_err(|_| OperationFailureKind::Invalid)?;
    let room = session
        .client()
        .get_room(&room_id)
        .ok_or(OperationFailureKind::NotFound)?;
    let item = if refresh.hydrate_root {
        Some(
            load_thread_root_projection_item_from_room(&room, key, own_user_id, &refresh.activity)
                .await?,
        )
    } else {
        None
    };
    let sdk_aggregate = resolve_thread_relation_aggregate(&room, &root_event_id)
        .await
        .map_err(|error| classify_thread_list_error(&error))?;
    let aggregate = authoritative_thread_aggregate_from_sdk(&sdk_aggregate);
    Ok(match item {
        Some(item) => ThreadRootProjectionRefreshResult::Hydrated { item, aggregate },
        None => ThreadRootProjectionRefreshResult::Aggregate(aggregate),
    })
}

fn thread_root_projection_dto_from_record(
    record: &ThreadRootProjectionRecord,
) -> ThreadRootProjectionDto {
    let state = if record.is_pending() {
        ThreadRootProjectionStateDto::Pending
    } else if let Some(item) = record.item() {
        ThreadRootProjectionStateDto::Ready {
            item: thread_root_item_with_authoritative_aggregate(item, &record.aggregate),
        }
    } else if let Some(failure_kind) = record.failure_kind() {
        ThreadRootProjectionStateDto::Failed { failure_kind }
    } else {
        ThreadRootProjectionStateDto::Pending
    };
    ThreadRootProjectionDto {
        root_event_id: record.activity.root_event_id.clone(),
        activity_event_id: record.activity.activity_event_id.clone(),
        activity_timestamp_ms: record.activity.activity_timestamp_ms,
        retain_without_reply: false,
        source: ThreadRootProjectionSourceDto::Hydration,
        state,
    }
}

fn thread_root_projection_pending_dto(
    activity: &ThreadRootProjectionActivity,
) -> ThreadRootProjectionDto {
    ThreadRootProjectionDto {
        root_event_id: activity.root_event_id.clone(),
        activity_event_id: activity.activity_event_id.clone(),
        activity_timestamp_ms: activity.activity_timestamp_ms,
        retain_without_reply: false,
        source: ThreadRootProjectionSourceDto::Hydration,
        state: ThreadRootProjectionStateDto::Pending,
    }
}

fn hydration_projection_event(
    key: &TimelineKey,
    projection: ThreadRootProjectionDto,
) -> TimelineEvent {
    TimelineEvent::ThreadRootProjection {
        key: key.clone(),
        projection,
    }
}

struct PreparedThreadRootHydration {
    activities_by_root: HashMap<String, ThreadRootProjectionActivity>,
    missing_activities: Vec<ThreadRootProjectionActivity>,
    canonical_root_event_ids: HashSet<String>,
    redacted_activity_event_ids: HashSet<String>,
    /// `None` refreshes the full bounded initial/reprojection window. A live
    /// batch supplies only roots whose canonical root/reply items changed.
    refresh_root_event_ids: Option<HashSet<String>>,
}

#[allow(clippy::too_many_arguments)]
async fn commit_prepared_thread_root_hydration_for_generation(
    service: &Arc<Mutex<ThreadRootProjectionService>>,
    replay_registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    generations: &Arc<TimelineActorGenerationGate>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    manager_tx: &mpsc::Sender<TimelineMessage>,
    event_tx: &broadcast::Sender<CoreEvent>,
    key: &TimelineKey,
    actor_generation: u64,
    own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
    prepared: PreparedThreadRootHydration,
) -> bool {
    let current_missing_activities = prepared
        .missing_activities
        .into_iter()
        .map(|activity| (activity.root_event_id.clone(), activity))
        .collect::<HashMap<_, _>>();
    let current_activities = prepared.activities_by_root;
    let canonical_root_event_ids = prepared.canonical_root_event_ids;
    let redacted_activity_event_ids = prepared.redacted_activity_event_ids;
    let refresh_root_event_ids = prepared.refresh_root_event_ids;
    let previous_tracked_activities = service
        .lock()
        .expect("thread-root projection service lock must not be poisoned")
        .active_activities(key.room_id());
    let manager_capacity_needed = !current_activities.is_empty()
        || !current_missing_activities.is_empty()
        || !previous_tracked_activities.is_empty();
    let refresh_permit = if manager_capacity_needed {
        let Ok(permit) = manager_tx.clone().reserve_owned().await else {
            return false;
        };
        Some(permit)
    } else {
        None
    };
    // Manager capacity is reserved first. The reducer permit is the final
    // await, so hydration/aggregate work can never hold reducer capacity while
    // a manager message that needs that same reducer is ahead of it in the
    // mailbox.
    let Ok(action_permit) = action_tx.clone().reserve_owned().await else {
        return false;
    };
    let Some(lease) = generations.try_acquire(key, actor_generation) else {
        return false;
    };
    let mut actions = vec![AppAction::ThreadRootProjectionsReconciled {
        room_id: key.room_id().to_owned(),
        activities: current_activities
            .values()
            .map(|activity| ThreadRootProjectionActivityState {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
            })
            .collect(),
    }];
    let mut events = Vec::new();
    let mut terminal_projections = Vec::new();
    let mut refreshes = Vec::new();
    let mut service_guard = service
        .lock()
        .expect("thread-root projection service lock must not be poisoned");
    service_guard.set_canonical_root_event_ids(key.room_id(), &canonical_root_event_ids);
    let mut affected_root_event_ids = previous_tracked_activities
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    affected_root_event_ids.extend(current_missing_activities.keys().cloned());
    for (root_event_id, previous_activity) in &previous_tracked_activities {
        if redacted_activity_event_ids.contains(&previous_activity.activity_event_id) {
            service_guard.invalidate_live_activity(
                key.room_id(),
                root_event_id,
                &previous_activity.activity_event_id,
            );
        }
    }
    let changed_root_event_ids = service_guard.reconcile_room_activities_with_affected(
        key.room_id(),
        &current_activities,
        &affected_root_event_ids,
    );
    for activity in current_activities.values() {
        let was_tracked = previous_tracked_activities.contains_key(&activity.root_event_id);
        let decision = service_guard.observe(activity.clone());
        match decision {
            ThreadRootProjectionDecision::StartFetch(activity) => {
                let canonical_root_active =
                    canonical_root_event_ids.contains(&activity.root_event_id);
                if !canonical_root_active {
                    actions.push(AppAction::ThreadRootProjectionObserved {
                        room_id: activity.room_id.clone(),
                        root_event_id: activity.root_event_id.clone(),
                        activity_event_id: activity.activity_event_id.clone(),
                        activity_timestamp_ms: activity.activity_timestamp_ms,
                    });
                    events.push(hydration_projection_event(
                        key,
                        thread_root_projection_pending_dto(&activity),
                    ));
                }
            }
            ThreadRootProjectionDecision::ActivityUpdated(record)
            | ThreadRootProjectionDecision::Existing(record) => {
                let canonical_root_active =
                    canonical_root_event_ids.contains(&activity.root_event_id);
                if !canonical_root_active {
                    actions.push(thread_root_projection_action_from_record(&record));
                    if record.is_pending() {
                        events.push(hydration_projection_event(
                            key,
                            thread_root_projection_dto_from_record(&record),
                        ));
                    } else {
                        terminal_projections.push(thread_root_projection_dto_from_record(&record));
                    }
                }
            }
            ThreadRootProjectionDecision::Retired => continue,
        }
        let cause = if !was_tracked {
            AggregateRefreshCause::InitialHydration
        } else if changed_root_event_ids.contains(&activity.root_event_id) {
            AggregateRefreshCause::SelectedActivity
        } else {
            AggregateRefreshCause::CanonicalBatch
        };
        let canonical_root_active = canonical_root_event_ids.contains(&activity.root_event_id);
        let should_refresh = !was_tracked
            || refresh_root_event_ids
                .as_ref()
                .is_none_or(|roots| roots.contains(&activity.root_event_id));
        if should_refresh
            && let Some(refresh) = service_guard.schedule_aggregate_refresh_with_canonical_root(
                activity,
                cause,
                true,
                canonical_root_active,
                false,
            )
        {
            refreshes.push(refresh);
        }
    }
    for (root_event_id, activity) in &previous_tracked_activities {
        if current_activities.contains_key(root_event_id)
            || refresh_root_event_ids
                .as_ref()
                .is_some_and(|roots| !roots.contains(root_event_id))
        {
            continue;
        }
        let canonical_root_active = canonical_root_event_ids.contains(root_event_id);
        let cause = if canonical_root_active {
            AggregateRefreshCause::CanonicalBatch
        } else {
            AggregateRefreshCause::Removal
        };
        if let Some(refresh) = service_guard.schedule_aggregate_refresh_with_canonical_root(
            activity,
            cause,
            false,
            canonical_root_active,
            false,
        ) {
            refreshes.push(refresh);
        }
    }
    action_permit.send(actions);
    emit_timeline_events_with_lease(event_tx, &lease, events);
    drop(service_guard);
    for projection in terminal_projections {
        let _ =
            emit_hydration_terminal_unless_replay_owned(event_tx, replay_registry, key, projection);
    }
    if let Some(permit) = refresh_permit {
        if !refreshes.is_empty() {
            permit.send(TimelineMessage::StartAggregateRefresh {
                key: key.clone(),
                actor_generation,
                own_user_id,
                refreshes,
            });
        }
    }
    true
}

fn thread_root_projection_action_from_record(record: &ThreadRootProjectionRecord) -> AppAction {
    if let Some(failure_kind) = record.failure_kind() {
        AppAction::ThreadRootProjectionFailed {
            room_id: record.activity.room_id.clone(),
            root_event_id: record.activity.root_event_id.clone(),
            activity_event_id: record.activity.activity_event_id.clone(),
            activity_timestamp_ms: record.activity.activity_timestamp_ms,
            failure_kind,
        }
    } else if record.item().is_some() {
        AppAction::ThreadRootProjectionReady {
            room_id: record.activity.room_id.clone(),
            root_event_id: record.activity.root_event_id.clone(),
            activity_event_id: record.activity.activity_event_id.clone(),
            activity_timestamp_ms: record.activity.activity_timestamp_ms,
        }
    } else {
        AppAction::ThreadRootProjectionObserved {
            room_id: record.activity.room_id.clone(),
            root_event_id: record.activity.root_event_id.clone(),
            activity_event_id: record.activity.activity_event_id.clone(),
            activity_timestamp_ms: record.activity.activity_timestamp_ms,
        }
    }
}

pub(super) fn seed_thread_summary_item(
    service: &Arc<Mutex<ThreadRootProjectionService>>,
    key: &TimelineKey,
    item: &TimelineItem,
) {
    if !matches!(key.kind, TimelineKind::Room { .. }) {
        return;
    }
    let TimelineItemId::Event { event_id } = &item.id else {
        return;
    };
    if item.thread_root.is_some() {
        return;
    }
    let Some(summary) = item.thread_summary.as_ref() else {
        return;
    };
    service
        .lock()
        .expect("thread-root projection service lock must not be poisoned")
        .seed_canonical_summary(key.room_id(), event_id, summary);
}

pub(super) fn seed_thread_summary_diff(
    service: &Arc<Mutex<ThreadRootProjectionService>>,
    key: &TimelineKey,
    diff: &crate::event::TimelineDiff,
) {
    match diff {
        crate::event::TimelineDiff::PushFront { item }
        | crate::event::TimelineDiff::PushBack { item }
        | crate::event::TimelineDiff::Insert { item, .. }
        | crate::event::TimelineDiff::Set { item, .. } => {
            seed_thread_summary_item(service, key, item);
        }
        crate::event::TimelineDiff::Reset { items } => {
            for item in items {
                seed_thread_summary_item(service, key, item);
            }
        }
        crate::event::TimelineDiff::Remove { .. }
        | crate::event::TimelineDiff::Truncate { .. }
        | crate::event::TimelineDiff::Clear => {}
    }
}

pub(super) fn overlay_thread_summary_item(
    service: &Arc<Mutex<ThreadRootProjectionService>>,
    key: &TimelineKey,
    item: &TimelineItem,
) -> TimelineItem {
    let TimelineItemId::Event { event_id } = &item.id else {
        return item.clone();
    };
    if item.thread_root.is_some() {
        return item.clone();
    }
    let Some(aggregate) = service
        .lock()
        .expect("thread-root projection service lock must not be poisoned")
        .current_aggregate(key.room_id(), event_id)
    else {
        return item.clone();
    };
    thread_root_item_with_authoritative_aggregate(item, &aggregate)
}

pub(super) fn overlay_thread_summary_diff(
    service: &Arc<Mutex<ThreadRootProjectionService>>,
    key: &TimelineKey,
    diff: &mut crate::event::TimelineDiff,
) {
    match diff {
        crate::event::TimelineDiff::PushFront { item }
        | crate::event::TimelineDiff::PushBack { item }
        | crate::event::TimelineDiff::Insert { item, .. }
        | crate::event::TimelineDiff::Set { item, .. } => {
            *item = overlay_thread_summary_item(service, key, item);
        }
        crate::event::TimelineDiff::Reset { items } => {
            for item in items {
                *item = overlay_thread_summary_item(service, key, item);
            }
        }
        crate::event::TimelineDiff::Remove { .. }
        | crate::event::TimelineDiff::Truncate { .. }
        | crate::event::TimelineDiff::Clear => {}
    }
}

pub(super) fn thread_root_item_with_authoritative_aggregate(
    item: &TimelineItem,
    aggregate: &crate::threads_list::AuthoritativeThreadAggregate,
) -> TimelineItem {
    let mut item = item.clone();
    let summary = item.thread_summary.get_or_insert(ThreadSummaryDto {
        reply_count: 0,
        latest_event_id: None,
        latest_sender: None,
        latest_sender_label: None,
        latest_body_preview: None,
        latest_timestamp_ms: None,
    });
    summary.reply_count = aggregate.reply_count;
    summary.latest_event_id = aggregate.latest_event_id.clone();
    summary.latest_sender = aggregate.latest_sender.clone();
    summary.latest_sender_label = aggregate.latest_sender_label.clone();
    summary.latest_body_preview = aggregate.latest_body_preview.clone();
    summary.latest_timestamp_ms = aggregate.latest_timestamp_ms;
    item
}

async fn load_thread_root_projection_item(
    session: &MatrixClientSession,
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    activity: &ThreadRootProjectionActivity,
) -> Result<TimelineItem, OperationFailureKind> {
    let room_id = matrix_sdk::ruma::RoomId::parse(activity.room_id.as_str())
        .map_err(|_| OperationFailureKind::Invalid)?;
    let room = session
        .client()
        .get_room(&room_id)
        .ok_or(OperationFailureKind::NotFound)?;
    load_thread_root_projection_item_from_room(&room, key, own_user_id, activity).await
}

async fn load_thread_root_projection_item_from_room(
    room: &matrix_sdk::Room,
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    activity: &ThreadRootProjectionActivity,
) -> Result<TimelineItem, OperationFailureKind> {
    let root_event_id = matrix_sdk::ruma::EventId::parse(activity.root_event_id.as_str())
        .map_err(|_| OperationFailureKind::Invalid)?;
    let loaded = room
        .load_or_fetch_event(&root_event_id, None)
        .await
        .map_err(|_| OperationFailureKind::Network)?;
    let raw: serde_json::Value =
        serde_json::from_str(loaded.raw().json().get()).map_err(|_| OperationFailureKind::Sdk)?;
    let sender_id = raw
        .get("sender")
        .and_then(serde_json::Value::as_str)
        .and_then(|sender| matrix_sdk::ruma::UserId::parse(sender).ok());
    let sender_profile = match sender_id {
        Some(sender_id) => room
            .get_member_no_sync(sender_id.as_ref())
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let sender_label = sender_profile
        .as_ref()
        .and_then(|member| member.display_name())
        .map(str::to_owned);
    let sender_avatar = sender_profile
        .as_ref()
        .and_then(|member| member.avatar_url())
        .map(|avatar_url| AvatarImage {
            mxc_uri: avatar_url.to_string(),
            thumbnail: AvatarThumbnailState::NotRequested,
        });
    let relation_events = match room.event_cache().await {
        Ok((cache, _drop_handles)) => cache
            .find_event_relations(&root_event_id, None)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|event| serde_json::from_str(event.raw().json().get()).ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    let context = ThreadRootProjectionRenderContext {
        sender_label,
        sender_avatar,
        reactions: reaction_groups_from_cached_relation_events(
            relation_events,
            root_event_id.as_str(),
            own_user_id,
        ),
    };
    thread_root_projection_item_from_raw_with_context(key, own_user_id, activity, raw, context)
        .ok_or(OperationFailureKind::Sdk)
}

fn thread_root_projection_activity_from_item(
    room_id: &str,
    item: &TimelineItem,
) -> Option<ThreadRootProjectionActivity> {
    if !is_attention_eligible_event(item) {
        return None;
    }
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    let root_event_id = item.thread_root.as_ref()?.trim();
    (!root_event_id.is_empty()).then(|| ThreadRootProjectionActivity {
        room_id: room_id.to_owned(),
        root_event_id: root_event_id.to_owned(),
        activity_event_id: event_id.clone(),
        activity_timestamp_ms: item.timestamp_ms,
        activity_sender: item.sender.clone(),
        activity_sender_label: item.sender_label.clone(),
        activity_body_preview: thread_root_activity_preview(item),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ThreadSummaryActivityObservation {
    Activity(ThreadRootProjectionActivity),
    Invalidated {
        root_event_id: String,
        activity_event_id: String,
    },
}

pub(super) fn thread_summary_affected_root_event_ids(
    key: &TimelineKey,
    before: &[TimelineItem],
    after: &[TimelineItem],
) -> HashSet<String> {
    if !matches!(key.kind, TimelineKind::Room { .. }) {
        return HashSet::new();
    }
    let collect = |items: &[TimelineItem]| {
        items
            .iter()
            .filter_map(|item| {
                let event_id = timeline_item_event_id(item)?.to_owned();
                let root_event_id = item
                    .thread_root
                    .as_deref()
                    .filter(|root| !root.trim().is_empty())
                    .map(str::to_owned)
                    .or_else(|| item.thread_summary.as_ref().map(|_| event_id.clone()))?;
                Some((event_id, (root_event_id, item.clone())))
            })
            .collect::<HashMap<_, _>>()
    };
    let before_by_event = collect(before);
    let after_by_event = collect(after);
    let mut affected = HashSet::new();
    for event_id in before_by_event.keys().chain(after_by_event.keys()) {
        let before = before_by_event.get(event_id);
        let after = after_by_event.get(event_id);
        if before == after {
            continue;
        }
        if let Some((root_event_id, _)) = before {
            affected.insert(root_event_id.clone());
        }
        if let Some((root_event_id, _)) = after {
            affected.insert(root_event_id.clone());
        }
    }
    affected
}

pub(super) fn thread_summary_observations_for_windows(
    key: &TimelineKey,
    before: &[TimelineItem],
    after: &[TimelineItem],
) -> Vec<ThreadSummaryActivityObservation> {
    let TimelineKind::Thread { root_event_id, .. } = &key.kind else {
        return Vec::new();
    };
    let collect = |items: &[TimelineItem]| {
        items
            .iter()
            .filter_map(|item| thread_root_projection_activity_from_item(key.room_id(), item))
            .filter(|activity| activity.root_event_id == *root_event_id)
            .map(|activity| (activity.activity_event_id.clone(), activity))
            .collect::<HashMap<_, _>>()
    };
    let before_by_event = collect(before);
    let after_by_event = collect(after);
    let after_items_by_event = after
        .iter()
        .filter_map(|item| Some((timeline_item_event_id(item)?.to_owned(), item)))
        .collect::<HashMap<_, _>>();
    let mut observations = Vec::new();
    for activity in after_by_event.values() {
        if before_by_event
            .get(&activity.activity_event_id)
            .is_none_or(|previous| previous != activity)
        {
            observations.push(ThreadSummaryActivityObservation::Activity(activity.clone()));
        }
    }
    for activity in before_by_event.values() {
        if !after_by_event.contains_key(&activity.activity_event_id)
            && after_items_by_event
                .get(&activity.activity_event_id)
                .is_some_and(|item| item.is_redacted)
        {
            observations.push(ThreadSummaryActivityObservation::Invalidated {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
            });
        }
    }
    observations.sort_by(|left, right| {
        let left_key = match left {
            ThreadSummaryActivityObservation::Activity(activity) => (
                1u8,
                activity.root_event_id.as_str(),
                activity.activity_event_id.as_str(),
            ),
            ThreadSummaryActivityObservation::Invalidated {
                root_event_id,
                activity_event_id,
            } => (0u8, root_event_id.as_str(), activity_event_id.as_str()),
        };
        let right_key = match right {
            ThreadSummaryActivityObservation::Activity(activity) => (
                1u8,
                activity.root_event_id.as_str(),
                activity.activity_event_id.as_str(),
            ),
            ThreadSummaryActivityObservation::Invalidated {
                root_event_id,
                activity_event_id,
            } => (0u8, root_event_id.as_str(), activity_event_id.as_str()),
        };
        left_key.cmp(&right_key)
    });
    observations
}

/// The exact Room items currently represented by the bounded display replay.
///
/// `navigation_items` deliberately has a wider lifetime than the UI's replay
/// window. It may therefore contain a latest reply that was not rendered. A
/// replay-known root must be reconciled against this context, never the whole
/// navigation cache, or an unrelated cached reply can clear the visible root.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ReplayKnownDisplayContext {
    pub(super) event_ids: HashSet<String>,
    pub(super) exact_thread_reply_pairs: HashSet<(String, String)>,
    pub(super) activity_range: Option<(u64, u64)>,
}

impl ReplayKnownDisplayContext {
    pub(super) fn from_display_items(display_items: &[TimelineItem]) -> Self {
        let event_ids = display_items
            .iter()
            .filter_map(timeline_item_event_id)
            .map(ToOwned::to_owned)
            .collect::<HashSet<_>>();
        let exact_thread_reply_pairs = display_items
            .iter()
            .filter_map(|item| {
                let root_event_id = item.thread_root.as_deref()?.trim();
                let reply_event_id = timeline_item_event_id(item)?.trim();
                (!root_event_id.is_empty() && !reply_event_id.is_empty())
                    .then(|| (root_event_id.to_owned(), reply_event_id.to_owned()))
            })
            .collect::<HashSet<_>>();
        Self {
            event_ids,
            exact_thread_reply_pairs,
            activity_range: replay_activity_timestamp_range(display_items),
        }
    }
}

/// Returns root snapshots already known to the actor but absent from the
/// bounded display context. This is not hydration: copying a root from
/// `navigation_items` must never call the SDK, paginate, or materialize a
/// viewport anchor.
#[cfg(test)]
fn known_thread_root_projections_for_replay(
    navigation_items: &[TimelineItem],
    replay_items: &[TimelineItem],
) -> Vec<ThreadRootProjectionDto> {
    known_thread_root_projections_for_display_context(
        navigation_items,
        &ReplayKnownDisplayContext::from_display_items(replay_items),
    )
}

pub(super) fn known_thread_root_projections_for_display_context(
    navigation_items: &[TimelineItem],
    display_context: &ReplayKnownDisplayContext,
) -> Vec<ThreadRootProjectionDto> {
    let Some((range_start_ms, range_end_ms)) = display_context.activity_range else {
        return Vec::new();
    };
    let mut emitted_root_event_ids = HashSet::new();
    let mut projections = navigation_items
        .iter()
        .filter_map(|item| {
            let root_event_id = timeline_item_event_id(item)?;
            if item.thread_root.is_some() || display_context.event_ids.contains(root_event_id) {
                return None;
            }
            let summary = item.thread_summary.as_ref()?;
            let activity_event_id = summary.latest_event_id.as_ref()?.trim();
            if activity_event_id.is_empty() {
                return None;
            }
            if display_context
                .exact_thread_reply_pairs
                .contains(&(root_event_id.to_owned(), activity_event_id.to_owned()))
            {
                return None;
            }
            let activity_timestamp_ms = summary.latest_timestamp_ms?;
            // The replay display range is inclusive: a summary on either
            // boundary belongs to the same visual window, never outside it.
            if activity_timestamp_ms < range_start_ms || activity_timestamp_ms > range_end_ms {
                return None;
            }
            if !emitted_root_event_ids.insert(root_event_id.to_owned()) {
                return None;
            }
            Some(ThreadRootProjectionDto {
                root_event_id: root_event_id.to_owned(),
                activity_event_id: activity_event_id.to_owned(),
                activity_timestamp_ms: Some(activity_timestamp_ms),
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready { item: item.clone() },
            })
        })
        .collect::<Vec<_>>();
    projections.sort_by(|left, right| {
        left.activity_timestamp_ms
            .cmp(&right.activity_timestamp_ms)
            .then_with(|| left.root_event_id.cmp(&right.root_event_id))
    });
    projections.truncate(ROOM_REPLAY_KNOWN_THREAD_ROOT_PROJECTIONS_MAX);
    projections
}

/// Returns the inclusive activity bounds represented by event-backed replay
/// rows. A replay with no timestamped event rows cannot place summary-only
/// roots chronologically, so it deliberately emits none.
fn replay_activity_timestamp_range(replay_items: &[TimelineItem]) -> Option<(u64, u64)> {
    replay_items
        .iter()
        .filter(|item| timeline_item_event_id(item).is_some())
        .filter_map(|item| item.timestamp_ms)
        .fold(None, |range, timestamp_ms| match range {
            Some((start, end)) => Some((start.min(timestamp_ms), end.max(timestamp_ms))),
            None => Some((timestamp_ms, timestamp_ms)),
        })
}

/// Derives the bounded replay candidates before entering an ownership group.
/// Only Room timelines have this out-of-band root snapshot behaviour.
pub(super) fn replay_known_candidates_for_display_items(
    key: &TimelineKey,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
) -> Vec<ThreadRootProjectionDto> {
    if !matches!(key.kind, TimelineKind::Room { .. }) {
        return Vec::new();
    }
    known_thread_root_projections_for_display_context(
        navigation_items,
        &ReplayKnownDisplayContext::from_display_items(display_items),
    )
}

#[cfg(test)]
pub(super) fn refresh_replay_known_root_projections(
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    key: &TimelineKey,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
) -> ReplayKnownThreadRootProjectionUpdate {
    refresh_replay_known_root_projections_with_display_context(
        registry,
        key,
        navigation_items,
        &ReplayKnownDisplayContext::from_display_items(display_items),
    )
}

#[cfg(test)]
fn refresh_replay_known_root_projections_with_display_context(
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    key: &TimelineKey,
    navigation_items: &[TimelineItem],
    display_context: &ReplayKnownDisplayContext,
) -> ReplayKnownThreadRootProjectionUpdate {
    let candidates = if matches!(key.kind, TimelineKind::Room { .. }) {
        known_thread_root_projections_for_display_context(navigation_items, display_context)
    } else {
        Vec::new()
    };
    registry
        .lock()
        .expect("replay-known root registry lock must not be poisoned")
        .replace(key, candidates)
}

#[cfg(test)]
pub(super) fn reconcile_replay_known_root_projections_after_navigation_update(
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    key: &TimelineKey,
    navigation_items: &[TimelineItem],
    display_context: &ReplayKnownDisplayContext,
) -> ReplayKnownThreadRootProjectionUpdate {
    registry
        .lock()
        .expect("replay-known root registry lock must not be poisoned")
        .reconcile_navigation(key, navigation_items, display_context)
}

fn replay_known_clear_projection(
    projection: ReplayKnownThreadRootProjection,
) -> ThreadRootProjectionDto {
    ThreadRootProjectionDto {
        root_event_id: projection.root_event_id,
        activity_event_id: projection.activity_event_id,
        activity_timestamp_ms: projection.activity_timestamp_ms,
        retain_without_reply: false,
        source: projection.source,
        state: ThreadRootProjectionStateDto::Cleared,
    }
}

#[cfg(test)]
fn emit_replay_known_root_projection_update(
    event_tx: &broadcast::Sender<CoreEvent>,
    key: &TimelineKey,
    update: ReplayKnownThreadRootProjectionUpdate,
) {
    for event in replay_known_timeline_events(key, update) {
        let _ = event_tx.send(CoreEvent::Timeline(event));
    }
}

#[cfg(test)]
fn replay_known_timeline_events(
    key: &TimelineKey,
    update: ReplayKnownThreadRootProjectionUpdate,
) -> Vec<TimelineEvent> {
    let mut events = Vec::with_capacity(update.stale.len() + update.ready.len());
    for projection in update.stale {
        events.push(TimelineEvent::ThreadRootProjection {
            key: key.clone(),
            projection: replay_known_clear_projection(projection),
        });
    }
    for projection in update.ready {
        events.push(TimelineEvent::ThreadRootProjection {
            key: key.clone(),
            projection,
        });
    }
    events
}

/// Builds a replay-known transition while the caller still owns the registry
/// mutex. When a root loses replay ownership, hand the retained terminal
/// hydration snapshot back after its source-scoped Clear so an exact canonical
/// reply slot continues to represent the complete root block. No lookup is
/// started here; the service is consulted read-only and this function never
/// awaits.
pub(super) fn replay_known_timeline_events_with_hydration_handoffs(
    key: &TimelineKey,
    registry: &mut ReplayKnownThreadRootProjectionRegistry,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    update: ReplayKnownThreadRootProjectionUpdate,
) -> Vec<TimelineEvent> {
    let mut events = Vec::with_capacity(update.stale.len() + update.ready.len() * 2);
    for projection in update.stale {
        let root_event_id = projection.root_event_id.clone();
        events.push(TimelineEvent::ThreadRootProjection {
            key: key.clone(),
            projection: replay_known_clear_projection(projection),
        });
        if registry.owns_root(key, &root_event_id) {
            continue;
        }
        // Reassert only a terminal that the frontend had already observed, or
        // one deliberately withheld while replay ownership was current. A
        // retained service terminal that was never emitted is not a UI source
        // and must remain silent after the replay Clear.
        let was_suppressed = registry.take_suppressed_hydration_terminal(key, &root_event_id);
        let was_emitted = registry.take_emitted_hydration_terminal(key, &root_event_id);
        if !was_suppressed && !was_emitted {
            continue;
        }
        let terminal_hydration = thread_root_projection_service
            .lock()
            .expect("thread-root projection service lock must not be poisoned")
            .terminal_record(key.room_id(), &root_event_id)
            .map(|record| thread_root_projection_dto_from_record(&record));
        if let Some(projection) = terminal_hydration {
            registry.mark_hydration_terminal_emitted(key, root_event_id);
            events.push(TimelineEvent::ThreadRootProjection {
                key: key.clone(),
                projection,
            });
        }
    }
    for projection in update.ready {
        events.push(TimelineEvent::ThreadRootProjection {
            key: key.clone(),
            projection,
        });
    }
    events
}

/// Delivers one hydration terminal only if a replay-owned snapshot has not
/// already won the same root. The replay registry lock covers both the
/// ownership decision and synchronous Core broadcast, so a replay Ready can
/// never appear between them and be overwritten by this hydration DTO.
///
/// The caller must finish reducer delivery before calling this helper. It does
/// no I/O and never awaits while the registry mutex is held.
fn emit_hydration_terminal_unless_replay_owned(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    key: &TimelineKey,
    projection: ThreadRootProjectionDto,
) -> bool {
    let mut registry = registry
        .lock()
        .expect("replay-known root registry lock must not be poisoned");
    if registry.owns_root(key, &projection.root_event_id) {
        registry.mark_hydration_terminal_suppressed(key, projection.root_event_id.clone());
        return false;
    }
    registry.mark_hydration_terminal_emitted(key, projection.root_event_id.clone());
    let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
        key: key.clone(),
        projection,
    }));
    true
}

fn thread_root_activity_preview(item: &TimelineItem) -> Option<String> {
    eligible_activity_preview(item)
}

/// Deserializes the public cache/network event just far enough to use the
/// same content-to-rendering functions as a canonical SDK timeline item. The
/// SDK's `EventTimelineItem` constructor is private, so deliberately do not
/// construct a second timeline merely for this projection.
fn message_projection_from_loaded_root_raw(raw: &serde_json::Value) -> Option<MessageProjection> {
    let content = raw.get("content")?.clone();
    match raw.get("type").and_then(serde_json::Value::as_str) {
        Some("m.room.message") => {
            let message = serde_json::from_value::<RoomMessageEventContent>(content).ok()?;
            Some(message_projection_from_msgtype(
                &message.msgtype,
                message.body(),
            ))
        }
        Some("m.sticker") => {
            let sticker = serde_json::from_value::<StickerEventContent>(content).ok()?;
            Some(sticker_projection_from_body(&sticker.body))
        }
        Some("m.room.encrypted") => Some(non_user_content_projection("Unable to decrypt message")),
        _ => None,
    }
}

/// Builds the normal reaction DTO from relation events already resident in the
/// event cache. This intentionally accepts only cached records and performs
/// no relation lookup over the network.
fn reaction_groups_from_cached_relation_events(
    events: Vec<serde_json::Value>,
    target_event_id: &str,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
) -> Vec<ReactionGroup> {
    let mut groups: BTreeMap<String, BTreeMap<String, Option<String>>> = BTreeMap::new();

    for event in events {
        if event.get("type").and_then(serde_json::Value::as_str) != Some("m.reaction") {
            continue;
        }
        let Some(sender) = event
            .get("sender")
            .and_then(serde_json::Value::as_str)
            .filter(|sender| !sender.is_empty())
        else {
            continue;
        };
        let Some(relates_to) = event.pointer("/content/m.relates_to") else {
            continue;
        };
        if relates_to
            .get("rel_type")
            .and_then(serde_json::Value::as_str)
            != Some("m.annotation")
            || relates_to
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                != Some(target_event_id)
        {
            continue;
        }
        let Some(key) = relates_to
            .get("key")
            .and_then(serde_json::Value::as_str)
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        let reaction_event_id = event
            .get("event_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        groups
            .entry(key.to_owned())
            .or_default()
            .entry(sender.to_owned())
            .or_insert(reaction_event_id);
    }

    groups
        .into_iter()
        .map(|(key, senders)| {
            let own_sender = own_user_id.map(matrix_sdk::ruma::UserId::as_str);
            ReactionGroup {
                key,
                count: senders.len().min(u32::MAX as usize) as u32,
                reacted_by_me: own_sender.is_some_and(|own| senders.contains_key(own)),
                my_reaction_event_id: own_sender
                    .and_then(|own| senders.get(own))
                    .cloned()
                    .flatten(),
                sender_preview: senders
                    .keys()
                    .take(3)
                    .cloned()
                    .map(|user_id| ReactionSender {
                        user_id,
                        display_label: None,
                    })
                    .collect(),
            }
        })
        .collect()
}

#[derive(Default)]
struct ThreadRootProjectionRenderContext {
    sender_label: Option<String>,
    sender_avatar: Option<AvatarImage>,
    reactions: Vec<ReactionGroup>,
}

/// Convert the cache/network event payload into a self-contained root DTO
/// without inserting it into the SDK timeline. `load_or_fetch_event` exposes a
/// public decrypted raw event, not the SDK-private `EventTimelineItem`; this
/// path therefore reuses the same message/media/formatted-body helpers as the
/// canonical conversion and augments it with cache-only profile/reaction data.
#[cfg(test)]
fn thread_root_projection_item_from_raw(
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    activity: &ThreadRootProjectionActivity,
    raw: serde_json::Value,
) -> Option<TimelineItem> {
    thread_root_projection_item_from_raw_with_context(
        key,
        own_user_id,
        activity,
        raw,
        ThreadRootProjectionRenderContext::default(),
    )
}

fn thread_root_projection_item_from_raw_with_context(
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    activity: &ThreadRootProjectionActivity,
    raw: serde_json::Value,
    context: ThreadRootProjectionRenderContext,
) -> Option<TimelineItem> {
    let event_id = raw.get("event_id")?.as_str()?.to_owned();
    if event_id != activity.root_event_id {
        return None;
    }
    let sender = raw
        .get("sender")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let timestamp_ms = raw
        .get("origin_server_ts")
        .and_then(serde_json::Value::as_u64);
    let content = raw.get("content").unwrap_or(&serde_json::Value::Null);
    let is_redacted = raw
        .get("unsigned")
        .and_then(|unsigned| unsigned.get("redacted_because"))
        .is_some();
    let message_projection = message_projection_from_loaded_root_raw(&raw);
    let body = message_projection
        .as_ref()
        .and_then(|projection| projection.body.clone())
        .or_else(|| {
            (raw.get("type").and_then(serde_json::Value::as_str) == Some("m.room.encrypted"))
                .then(|| "Unable to decrypt message".to_owned())
        });
    let notice_i18n = message_projection
        .as_ref()
        .and_then(|projection| projection.notice_i18n.clone());
    let message_kind = message_projection
        .as_ref()
        .map(|projection| projection.message_kind)
        .unwrap_or_default();
    let spoiler_spans = message_projection
        .as_ref()
        .map(|projection| projection.spoiler_spans.clone())
        .unwrap_or_default();
    let media = message_projection
        .as_ref()
        .and_then(|projection| projection.media.clone());
    let formatted = message_projection
        .as_ref()
        .and_then(|projection| projection.formatted.clone());
    let actionable_body = (!is_redacted)
        .then(|| {
            message_projection
                .as_ref()
                .filter(|projection| projection.body_is_user_content)
                .and_then(|projection| projection.body.as_deref())
        })
        .flatten();
    let id = TimelineItemId::Event {
        event_id: event_id.clone(),
    };
    let thread_summary = thread_summary_from_loaded_root_raw(&raw);

    Some(TimelineItem {
        id,
        sender: sender.clone(),
        sender_label: context.sender_label,
        sender_avatar: context.sender_avatar,
        body: body.clone(),
        notice_i18n,
        message_kind,
        spoiler_spans,
        timestamp_ms,
        in_reply_to_event_id: content
            .get("m.relates_to")
            .and_then(|relation| relation.get("m.in_reply_to"))
            .and_then(|reply| reply.get("event_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        formatted: formatted.clone(),
        reply_quote: None,
        thread_root: None,
        thread_summary,
        media: media.clone(),
        link_previews: None,
        link_ranges: link_ranges_for_message_projection(body.as_deref(), formatted.as_ref()),
        reactions: context.reactions,
        can_react: !is_redacted
            && timeline_content_is_renderable(body.as_deref(), media.as_ref(), formatted.as_ref()),
        is_redacted,
        // A loaded old root is deliberately visible even if it is a
        // non-message event; the terminal state must be observable rather
        // than triggering another history fetch.
        is_hidden: false,
        can_redact: !is_redacted
            && timeline_content_is_renderable(body.as_deref(), media.as_ref(), formatted.as_ref())
            && own_user_id
                .zip(sender.as_deref())
                .is_some_and(|(own, event_sender)| own.as_str() == event_sender),
        is_edited: false,
        can_edit: !is_redacted
            && actionable_body.is_some()
            && own_user_id
                .zip(sender.as_deref())
                .is_some_and(|(own, event_sender)| own.as_str() == event_sender),
        unable_to_decrypt: (raw.get("type").and_then(serde_json::Value::as_str)
            == Some("m.room.encrypted"))
        .then_some(TimelineUnableToDecrypt {
            session_id: None,
            reason: TimelineUnableToDecryptReason::Unknown,
            can_request_keys: false,
            recovery_stage: None,
            recovery_guidance: None,
        }),
        request_state: None,
        actions: message_actions_for_timeline_item(
            key.room_id(),
            &TimelineItemId::Event { event_id },
            actionable_body,
            media.is_some(),
            is_redacted,
        ),
        send_state: None,
    })
}

fn thread_summary_from_loaded_root_raw(raw: &serde_json::Value) -> Option<ThreadSummaryDto> {
    let summary = raw.get("unsigned")?.get("m.relations")?.get("m.thread")?;
    let latest = summary.get("latest_event");
    Some(ThreadSummaryDto {
        reply_count: summary
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(0),
        latest_event_id: latest
            .and_then(|event| event.get("event_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        latest_sender: latest
            .and_then(|event| event.get("sender"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        latest_sender_label: None,
        latest_body_preview: latest
            .and_then(|event| event.get("content"))
            .and_then(|content| content.get("body"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        latest_timestamp_ms: latest
            .and_then(|event| event.get("origin_server_ts"))
            .and_then(serde_json::Value::as_u64),
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ThreadAttentionCounters {
    pub(super) notification_count: u64,
    pub(super) highlight_count: u64,
    pub(super) live_event_marker_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ThreadAttentionObservation {
    Live,
    Backfill,
    Replay,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ThreadAttentionBatchProvenance {
    event_observations: HashMap<String, ThreadAttentionObservation>,
}

pub(super) fn gap_repair_projections_from_sdk_diffs(
    diffs: &[eyeball_im::VectorDiff<Arc<SdkTimelineItem>>],
) -> BTreeSet<CausalProjectionId> {
    let mut projections = BTreeSet::new();
    let mut observe = |item: &Arc<SdkTimelineItem>| {
        if let Some(projection) = item
            .as_event()
            .and_then(EventTimelineItem::gap_repair_projection)
        {
            projections.insert(CausalProjectionId::decode_transport(projection));
        }
    };
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::PushFront { value }
            | eyeball_im::VectorDiff::PushBack { value }
            | eyeball_im::VectorDiff::Insert { value, .. }
            | eyeball_im::VectorDiff::Set { value, .. } => observe(value),
            eyeball_im::VectorDiff::Reset { values }
            | eyeball_im::VectorDiff::Append { values } => {
                for value in values {
                    observe(value);
                }
            }
            eyeball_im::VectorDiff::Remove { .. }
            | eyeball_im::VectorDiff::Truncate { .. }
            | eyeball_im::VectorDiff::Clear
            | eyeball_im::VectorDiff::PopFront
            | eyeball_im::VectorDiff::PopBack => {}
        }
    }
    projections
}

fn thread_attention_observation_from_event_origin(
    origin: Option<EventItemOrigin>,
) -> ThreadAttentionObservation {
    match origin {
        Some(EventItemOrigin::Sync) => ThreadAttentionObservation::Live,
        Some(EventItemOrigin::Pagination) => ThreadAttentionObservation::Backfill,
        Some(EventItemOrigin::Cache) | Some(EventItemOrigin::Local) | None => {
            ThreadAttentionObservation::Replay
        }
    }
}

impl ThreadAttentionBatchProvenance {
    pub(super) fn from_sdk_diffs(diffs: &[eyeball_im::VectorDiff<Arc<SdkTimelineItem>>]) -> Self {
        let mut provenance = Self::default();
        for diff in diffs {
            match diff {
                eyeball_im::VectorDiff::PushFront { value }
                | eyeball_im::VectorDiff::PushBack { value }
                | eyeball_im::VectorDiff::Insert { value, .. }
                | eyeball_im::VectorDiff::Set { value, .. } => {
                    provenance.observe_sdk_item(value, None);
                }
                // Reset and Append are replay/full-window shapes. Even if an
                // individual SDK item retains Sync origin, this delivery is
                // not evidence that it first arrived live in this actor.
                eyeball_im::VectorDiff::Reset { values }
                | eyeball_im::VectorDiff::Append { values } => {
                    for value in values {
                        provenance
                            .observe_sdk_item(value, Some(ThreadAttentionObservation::Replay));
                    }
                }
                eyeball_im::VectorDiff::Remove { .. }
                | eyeball_im::VectorDiff::Truncate { .. }
                | eyeball_im::VectorDiff::Clear
                | eyeball_im::VectorDiff::PopFront
                | eyeball_im::VectorDiff::PopBack => {}
            }
        }
        provenance
    }

    fn from_timeline_items(
        items: &[TimelineItem],
        observation: ThreadAttentionObservation,
    ) -> Self {
        let event_observations = items
            .iter()
            .filter_map(|item| match &item.id {
                TimelineItemId::Event { event_id } => Some((event_id.clone(), observation)),
                TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
            })
            .collect();
        Self { event_observations }
    }

    fn observe_sdk_item(
        &mut self,
        item: &Arc<SdkTimelineItem>,
        forced: Option<ThreadAttentionObservation>,
    ) {
        let Some(event) = item.as_event() else {
            return;
        };
        let Some(event_id) = event.event_id() else {
            return;
        };
        let observation = forced
            .unwrap_or_else(|| thread_attention_observation_from_event_origin(event.origin()));
        self.event_observations
            .entry(event_id.to_string())
            .and_modify(|existing| {
                if *existing != observation {
                    *existing = ThreadAttentionObservation::Replay;
                }
            })
            .or_insert(observation);
    }

    pub(super) fn observation_for(&self, event_id: &str) -> Option<ThreadAttentionObservation> {
        self.event_observations.get(event_id).copied()
    }
}

#[derive(Debug, Default)]
pub(super) struct ThreadAttentionTracker {
    pub(super) receipt_event_id: Option<String>,
    pub(super) observed_reply_event_ids: HashSet<String>,
    pub(super) attention_event_ids: HashSet<String>,
    pub(super) counts: ThreadAttentionCounters,
}

impl ThreadAttentionTracker {
    pub(super) fn hydrate(
        key: &TimelineKey,
        items: &[TimelineItem],
        own_user_id: Option<&str>,
        receipt_event_id: Option<String>,
    ) -> Self {
        let mut tracker = Self {
            receipt_event_id,
            ..Self::default()
        };
        tracker.observe_without_increment(key, items);
        if let (TimelineKind::Thread { root_event_id, .. }, Some(receipt_event_id)) =
            (&key.kind, tracker.receipt_event_id.as_deref())
        {
            if let Some(receipt_position) = items.iter().position(|item| {
                matches!(
                    &item.id,
                    TimelineItemId::Event { event_id } if event_id == receipt_event_id
                )
            }) {
                tracker.attention_event_ids.extend(
                    items
                        .iter()
                        .skip(receipt_position.saturating_add(1))
                        .filter_map(|item| {
                            matching_remote_thread_reply_event_id(item, root_event_id, own_user_id)
                                .map(str::to_owned)
                        }),
                );
                tracker.refresh_counts();
            }
        }
        tracker
    }

    pub(super) fn reconcile(
        &mut self,
        key: &TimelineKey,
        items: &[TimelineItem],
        own_user_id: Option<&str>,
        observation: ThreadAttentionObservation,
    ) -> Option<AppAction> {
        let provenance = ThreadAttentionBatchProvenance::from_timeline_items(items, observation);
        self.reconcile_batch(key, items, own_user_id, &provenance)
    }

    pub(super) fn reconcile_batch(
        &mut self,
        key: &TimelineKey,
        items: &[TimelineItem],
        own_user_id: Option<&str>,
        provenance: &ThreadAttentionBatchProvenance,
    ) -> Option<AppAction> {
        let TimelineKind::Thread { root_event_id, .. } = &key.kind else {
            return None;
        };
        let previous = self.counts;
        let eligible_reply_event_ids = items
            .iter()
            .filter(|item| is_attention_eligible_event(item))
            .filter_map(|item| matching_thread_reply_event_id(item, root_event_id))
            .collect::<HashSet<_>>();
        self.attention_event_ids
            .retain(|event_id| eligible_reply_event_ids.contains(event_id.as_str()));
        let event_positions = items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| match &item.id {
                TimelineItemId::Event { event_id } => Some((event_id.as_str(), position)),
                TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
            })
            .collect::<HashMap<_, _>>();
        let receipt_position = self
            .receipt_event_id
            .as_deref()
            .and_then(|receipt_event_id| event_positions.get(receipt_event_id).copied());
        if let Some(receipt_position) = receipt_position {
            self.attention_event_ids.retain(|event_id| {
                event_positions
                    .get(event_id.as_str())
                    .is_none_or(|position| *position > receipt_position)
            });
        }

        for (position, item) in items.iter().enumerate() {
            if !is_attention_eligible_event(item) {
                continue;
            }
            let Some(stable_event_id) = matching_thread_reply_event_id(item, root_event_id) else {
                continue;
            };
            let Some(observation) = provenance.observation_for(stable_event_id) else {
                continue;
            };
            let is_authoritatively_unread =
                receipt_position.is_some_and(|receipt_position| position > receipt_position);
            let may_add_attention = observation == ThreadAttentionObservation::Live
                || (observation == ThreadAttentionObservation::Replay && is_authoritatively_unread);
            if !may_add_attention {
                self.observed_reply_event_ids
                    .insert(stable_event_id.to_owned());
                continue;
            }
            if self.observed_reply_event_ids.contains(stable_event_id) {
                continue;
            }
            if own_user_id.is_some_and(|own_user_id| item.sender.as_deref() == Some(own_user_id)) {
                self.observed_reply_event_ids
                    .insert(stable_event_id.to_owned());
                continue;
            }
            if receipt_position.is_some_and(|receipt_position| position <= receipt_position) {
                self.observed_reply_event_ids
                    .insert(stable_event_id.to_owned());
                continue;
            }
            self.observed_reply_event_ids
                .insert(stable_event_id.to_owned());
            self.attention_event_ids.insert(stable_event_id.to_owned());
        }

        self.refresh_counts();
        (self.counts != previous)
            .then(|| thread_attention_action(self.counts, key))
            .flatten()
    }

    pub(super) fn acknowledge(
        &mut self,
        key: &TimelineKey,
        items: &[TimelineItem],
        event_id: String,
    ) -> Option<AppAction> {
        let TimelineKind::Thread { root_event_id, .. } = &key.kind else {
            return None;
        };
        let eligible_reply_event_ids = items
            .iter()
            .filter(|item| is_attention_eligible_event(item))
            .filter_map(|item| matching_thread_reply_event_id(item, root_event_id))
            .collect::<HashSet<_>>();
        self.attention_event_ids.retain(|attention_event_id| {
            eligible_reply_event_ids.contains(attention_event_id.as_str())
        });
        self.receipt_event_id = Some(event_id.clone());
        let positions = items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| match &item.id {
                TimelineItemId::Event { event_id } => Some((event_id.as_str(), position)),
                TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
            })
            .collect::<HashMap<_, _>>();
        let receipt_position = positions.get(event_id.as_str()).copied();
        self.attention_event_ids.retain(|attention_event_id| {
            match (
                receipt_position,
                positions.get(attention_event_id.as_str()).copied(),
            ) {
                (Some(receipt_position), Some(attention_position)) => {
                    attention_position > receipt_position
                }
                // A receipt outside the retained window is authoritative as a
                // future baseline, but its ordering relative to retained
                // attention is unknown. Preserve the count until the SDK gives
                // us a correlatable canonical position.
                (None, _) => true,
                (Some(_), None) => false,
            }
        });
        self.refresh_counts();
        thread_attention_action(self.counts, key)
    }

    fn observe_without_increment(&mut self, key: &TimelineKey, items: &[TimelineItem]) {
        let TimelineKind::Thread { root_event_id, .. } = &key.kind else {
            return;
        };
        self.observed_reply_event_ids.extend(
            items
                .iter()
                .filter(|item| is_attention_eligible_event(item))
                .filter_map(|item| {
                    matching_thread_reply_event_id(item, root_event_id).map(str::to_owned)
                }),
        );
    }

    fn refresh_counts(&mut self) {
        let count = self.attention_event_ids.len() as u64;
        self.counts.notification_count = count;
        self.counts.live_event_marker_count = count;
    }
}

impl TimelineActor {
    /// Detect Room thread replies whose root is not present in the canonical
    /// SDK item window. The projection service is deliberately out-of-band:
    /// this method never creates a VectorDiff, calls Room pagination, or
    /// asks the viewport/anchor path to materialize an event.
    pub(super) async fn maybe_hydrate_missing_thread_roots(
        &mut self,
        refresh_root_event_ids: Option<HashSet<String>>,
    ) {
        if !matches!(self.key.kind, TimelineKind::Room { .. }) {
            return;
        }

        let activities_by_root = self
            .navigation_items
            .iter()
            .filter_map(|item| thread_root_projection_activity_from_item(self.key.room_id(), item))
            .fold(HashMap::new(), |mut selected, activity| {
                let should_replace = selected
                    .get(&activity.root_event_id)
                    .is_none_or(|existing| activity_is_newer(&activity, existing));
                if should_replace {
                    selected.insert(activity.root_event_id.clone(), activity);
                }
                selected
            });
        let missing_activities = activities_by_root
            .values()
            .filter(|activity| !self.timeline_contains_event_id(&activity.root_event_id))
            .cloned()
            .collect();
        let canonical_root_event_ids = self
            .navigation_items
            .iter()
            .filter(|item| item.thread_root.is_none() && item.thread_summary.is_some())
            .filter_map(timeline_item_event_id)
            .map(ToOwned::to_owned)
            .collect();
        let redacted_activity_event_ids = self
            .navigation_items
            .iter()
            .filter(|item| item.is_redacted)
            .filter_map(timeline_item_event_id)
            .map(ToOwned::to_owned)
            .collect();
        let _ = commit_prepared_thread_root_hydration_for_generation(
            &self.thread_root_projection_service,
            &self.replay_known_thread_root_projections,
            &self.timeline_actor_generations,
            &self.action_tx,
            &self.manager_tx,
            &self.event_tx,
            &self.key,
            self.actor_generation,
            self.own_user_id.clone(),
            PreparedThreadRootHydration {
                activities_by_root,
                missing_activities,
                canonical_root_event_ids,
                redacted_activity_event_ids,
                refresh_root_event_ids,
            },
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_source::item_body;

    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::future::Future;

    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::Poll;
    use std::time::Duration;

    use koushi_state::{AppAction, ComposerFormattingOptions, OperationFailureKind};

    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_ui::timeline::{
        EventItemOrigin, TimelineDetails, TimelineEventItemId, TimelineItemContent,
    };
    use tokio::sync::{broadcast, mpsc};

    use crate::account_work::AccountWorkScheduler;

    use crate::command::TimelineCommand;
    use crate::event::{
        CoreEvent, LinkPreview, LinkPreviewState, PaginationDirection, PaginationState,
        ReactionGroup, ReactionSender, ThreadRootProjectionDto, ThreadRootProjectionSourceDto,
        ThreadRootProjectionStateDto, ThreadSummaryDto, TimelineDiff, TimelineEvent, TimelineItem,
        TimelineItemId, TimelineMediaKind, TimelineMessageActions, TimelineViewportObservation,
    };
    use crate::executor;

    use crate::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};
    use crate::link_preview::LinkPreviewContext;

    use crate::live_tail_freshness::LiveTailRefreshCoordinator;

    use crate::threads_list::{
        AggregateRefreshCause, ThreadRootProjectionActivity, ThreadRootProjectionDecision,
        ThreadRootProjectionService,
    };

    use std::future::poll_fn;

    use matrix_sdk::ruma::{OwnedUserId, uint};

    use super::super::actor::{
        ThreadSummaryProjectionIngress, TimelineActorHandle, TimelineActorMessage,
        emit_app_action_reliable,
    };
    use super::super::display_projection::{
        DisplayProjectionState, apply_non_sdk_item_set_diffs_to_display_items,
        apply_timeline_diffs_to_items,
    };
    use super::super::item_projection::{
        apply_ignored_sender_suppression, megolm_session_fingerprint,
        thread_root_from_original_json, thread_summary_from_sdk, timeline_item_event_id,
        timeline_item_should_be_hidden_for_key,
    };
    use super::super::manager::{TimelineManagerActor, TimelineMessage};
    use super::super::navigation::{
        InitialItemsRequestIdentity, ROOM_REPLAY_INITIAL_ITEMS_MAX, TimelineActorGenerationGate,
        accept_projection_ack_for_active_actor,
        emit_initial_items_and_reconcile_replay_known_for_generation,
        emit_initial_items_and_reconcile_replay_known_for_generation_with_test_hook,
        emit_items_updated_and_reconcile_replay_known_for_generation,
        emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation,
        emit_timeline_events_for_generation, emit_timeline_events_with_lease,
        replay_initial_items_window, replay_projection_request_id,
    };
    use super::super::outbound_send::{
        SendEnqueueWorkerSupervisor, SharedSendCompletionCoordinator, SubmissionAdmissionLedger,
        TimelineSendTerminalIngress, newest_provable_receipt_event_id,
        thread_activity_observed_action, thread_activity_observed_action_for_batch,
    };
    use super::super::read_state::ReadWorkerSupervisor;

    use super::super::test_support::{
        fake_rid, focused_key, live_tail_test_manager, replay_projection_services, room_key,
        test_timeline_actor_handle, thread_key, timeline_item,
    };
    use crate::threads_list::AuthoritativeThreadAggregate;

    use super::{
        JAVASCRIPT_SAFE_INTEGER_MAX, PreparedThreadRootHydration,
        ROOM_REPLAY_KNOWN_THREAD_ROOT_PROJECTIONS_MAX, ReplayKnownDisplayContext,
        ReplayKnownThreadRootProjection, ReplayKnownThreadRootProjectionRegistry,
        ThreadAttentionBatchProvenance, ThreadAttentionCounters, ThreadAttentionObservation,
        ThreadAttentionTracker, ThreadRootProjectionFetchRegistry,
        commit_prepared_thread_root_hydration_for_generation,
        emit_hydration_terminal_unless_replay_owned, emit_replay_known_root_projection_update,
        known_thread_root_projections_for_replay, overlay_thread_summary_diff,
        reaction_groups_from_cached_relation_events,
        reconcile_replay_known_root_projections_after_navigation_update,
        refresh_replay_known_root_projections,
        refresh_replay_known_root_projections_with_display_context, replay_known_timeline_events,
        replay_known_timeline_events_with_hydration_handoffs,
        thread_attention_observation_from_event_origin,
        thread_root_item_with_authoritative_aggregate, thread_root_projection_activity_from_item,
        thread_root_projection_dto_from_record, thread_root_projection_item_from_raw,
        thread_summary_affected_root_event_ids,
    };

    async fn assert_pending_on_first_poll<F: Future>(
        mut future: Pin<&mut F>,
        context: &'static str,
    ) {
        poll_fn(move |cx| match future.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(_) => panic!("{context} must be pending on its full channel"),
        })
        .await;
    }

    #[test]
    fn thread_activity_promotion_requires_a_matching_event_backed_reply() {
        let key = thread_key();
        let matching = thread_reply_item("$reply:test", "@b:test", "$root:test");
        assert_eq!(
            thread_activity_observed_action(&key, std::slice::from_ref(&matching)),
            Some(AppAction::ThreadActivityObserved {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
            })
        );
        let live_batch = ThreadAttentionBatchProvenance::from_timeline_items(
            std::slice::from_ref(&matching),
            ThreadAttentionObservation::Live,
        );
        assert_eq!(
            thread_activity_observed_action_for_batch(
                &key,
                std::slice::from_ref(&matching),
                &live_batch,
            ),
            Some(AppAction::ThreadActivityObserved {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
            })
        );
        assert_eq!(
            thread_activity_observed_action_for_batch(
                &key,
                std::slice::from_ref(&matching),
                &ThreadAttentionBatchProvenance::default(),
            ),
            None
        );

        let mut local_echo = matching;
        local_echo.id = TimelineItemId::Transaction {
            transaction_id: "txn".to_owned(),
        };
        assert_eq!(thread_activity_observed_action(&key, &[local_echo]), None);
        assert_eq!(
            thread_activity_observed_action(
                &key,
                &[thread_reply_item(
                    "$other:test",
                    "@b:test",
                    "$other-root:test",
                )],
            ),
            None
        );
        assert_eq!(
            thread_activity_observed_action(
                &room_key(),
                &[thread_reply_item("$reply:test", "@b:test", "$root:test",)]
            ),
            None
        );
    }

    #[test]
    fn resubscribe_replay_caps_room_timeline_to_live_window() {
        let key = room_key();
        let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
            .map(|index| {
                timeline_item(
                    &format!("$event-{index}:test"),
                    Some("body"),
                    "@bob:test",
                    false,
                )
            })
            .collect::<Vec<_>>();

        let replay = replay_initial_items_window(
            &key.kind,
            &items,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
        );

        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
        assert_eq!(
            replay.first().and_then(timeline_item_event_id),
            Some("$event-25:test")
        );
        let expected_last = format!("$event-{}:test", ROOM_REPLAY_INITIAL_ITEMS_MAX + 24);
        assert_eq!(
            replay.last().and_then(timeline_item_event_id),
            Some(expected_last.as_str())
        );
    }

    #[test]
    fn replay_display_window_emits_a_known_ready_root_without_a_reply_or_fetch() {
        let key = room_key();
        let root_event_id = "$old-root:test";
        let activity_event_id = "$summary-only-activity:test";
        let activity_timestamp_ms = 1_700_000_100_000;
        let mut root = timeline_item(root_event_id, Some("old root"), "@alice:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some(activity_event_id.to_owned()),
            latest_sender: Some("@bob:test".to_owned()),
            latest_sender_label: Some("Bob".to_owned()),
            latest_body_preview: Some("latest threaded activity".to_owned()),
            latest_timestamp_ms: Some(activity_timestamp_ms),
        });

        // The actor's canonical navigation cache still knows the root, but a
        // Room re-subscription at the live edge deliberately replays only the
        // final 120 rows to the display store. This server/SDK-shaped case has
        // a root summary but no standalone canonical reply item at all.
        let mut navigation_items = vec![root.clone()];
        navigation_items.extend((0..ROOM_REPLAY_INITIAL_ITEMS_MAX).map(|index| {
            let mut item = timeline_item(
                &format!("$ordinary-{index}:test"),
                Some("ordinary"),
                "@alice:test",
                false,
            );
            item.timestamp_ms = Some(activity_timestamp_ms + index as u64);
            item
        }));

        let replay = replay_initial_items_window(
            &key.kind,
            &navigation_items,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
        );
        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
        assert!(
            navigation_items
                .iter()
                .any(|item| timeline_item_event_id(item) == Some(root_event_id)),
            "the canonical cache is intentionally richer than the display replay"
        );
        assert!(
            !replay
                .iter()
                .any(|item| timeline_item_event_id(item) == Some(root_event_id)),
            "the old root must be outside the emitted replay window"
        );
        assert!(
            !replay.iter().any(|item| item.thread_root.is_some()),
            "the replay intentionally has no standalone canonical reply row"
        );

        let replay_projections =
            known_thread_root_projections_for_replay(&navigation_items, &replay);
        assert!(matches!(
            replay_projections.as_slice(),
            [ThreadRootProjectionDto {
                root_event_id,
                activity_event_id: emitted_activity_event_id,
                activity_timestamp_ms: Some(emitted_activity_timestamp_ms),
                retain_without_reply: true,
                state: ThreadRootProjectionStateDto::Ready { item },
                ..
            }]
                if root_event_id == "$old-root:test"
                    && emitted_activity_event_id == activity_event_id
                    && *emitted_activity_timestamp_ms == activity_timestamp_ms
                    && *item == root
        ));

        assert!(known_thread_root_projections_for_replay(&navigation_items, &[root]).is_empty());

        let source = include_str!("thread_projection.rs");
        let known_replay_helper = source
            .split("fn known_thread_root_projections_for_replay")
            .nth(1)
            .expect("known replay helper must exist")
            .split("fn thread_root_activity_preview")
            .next()
            .expect("known replay helper boundary");
        for forbidden in [
            "load_or_fetch_event",
            "paginate_backwards",
            "Paginate",
            "RestoreTimelineAnchor",
        ] {
            assert!(
                !known_replay_helper.contains(forbidden),
                "known replay snapshots must not start {forbidden}"
            );
        }
    }

    #[test]
    fn replay_known_roots_stay_inside_the_display_activity_range_inclusively() {
        let root_with_summary =
            |root_event_id: &str, activity_event_id: &str, activity_timestamp_ms: u64| {
                let mut root = timeline_item(root_event_id, Some("root"), "@alice:test", false);
                root.thread_summary = Some(ThreadSummaryDto {
                    reply_count: 1,
                    latest_event_id: Some(activity_event_id.to_owned()),
                    latest_sender: None,
                    latest_sender_label: None,
                    latest_body_preview: None,
                    latest_timestamp_ms: Some(activity_timestamp_ms),
                });
                root
            };
        let mut range_start = timeline_item("$range-start:test", Some("start"), "@a:test", false);
        range_start.timestamp_ms = Some(100);
        let mut range_end = timeline_item("$range-end:test", Some("end"), "@a:test", false);
        range_end.timestamp_ms = Some(200);
        let mut canonical_reply =
            timeline_item("$canonical-reply:test", Some("reply"), "@b:test", false);
        canonical_reply.timestamp_ms = Some(150);
        canonical_reply.thread_root = Some("$exact-root:test".to_owned());
        let replay = vec![range_start, canonical_reply, range_end];
        let navigation = vec![
            root_with_summary("$below:test", "$below-activity:test", 99),
            root_with_summary("$at-start:test", "$at-start-activity:test", 100),
            root_with_summary("$tie-b:test", "$tie-b-activity:test", 150),
            root_with_summary("$tie-a:test", "$tie-a-activity:test", 150),
            root_with_summary("$at-end:test", "$at-end-activity:test", 200),
            root_with_summary("$above:test", "$above-activity:test", 201),
            root_with_summary("$exact-root:test", "$canonical-reply:test", 150),
        ];

        let projections = known_thread_root_projections_for_replay(&navigation, &replay);

        assert_eq!(
            projections
                .iter()
                .map(|projection| projection.root_event_id.as_str())
                .collect::<Vec<_>>(),
            [
                "$at-start:test",
                "$tie-a:test",
                "$tie-b:test",
                "$at-end:test"
            ],
            "summary activity bounds are inclusive, ties are root-ID deterministic, and an exact reply owns its root"
        );
    }

    #[test]
    fn replay_known_roots_are_capped_deterministically() {
        let mut replay_start = timeline_item("$range-start:test", Some("start"), "@a:test", false);
        replay_start.timestamp_ms = Some(100);
        let mut replay_end = timeline_item("$range-end:test", Some("end"), "@a:test", false);
        replay_end.timestamp_ms = Some(200);
        let navigation = (0..(ROOM_REPLAY_KNOWN_THREAD_ROOT_PROJECTIONS_MAX + 8))
            .rev()
            .map(|index| {
                let mut root = timeline_item(
                    &format!("$root-{index:03}:test"),
                    Some("root"),
                    "@alice:test",
                    false,
                );
                root.thread_summary = Some(ThreadSummaryDto {
                    reply_count: 1,
                    latest_event_id: Some(format!("$activity-{index:03}:test")),
                    latest_sender: None,
                    latest_sender_label: None,
                    latest_body_preview: None,
                    latest_timestamp_ms: Some(150),
                });
                root
            })
            .collect::<Vec<_>>();

        let projections =
            known_thread_root_projections_for_replay(&navigation, &[replay_start, replay_end]);

        assert_eq!(
            projections.len(),
            ROOM_REPLAY_KNOWN_THREAD_ROOT_PROJECTIONS_MAX,
            "a bounded 120-item replay cannot grow without a deterministic root cap"
        );
        assert_eq!(
            projections
                .first()
                .map(|projection| projection.root_event_id.as_str()),
            Some("$root-000:test")
        );
        let expected_last = format!(
            "$root-{:03}:test",
            ROOM_REPLAY_KNOWN_THREAD_ROOT_PROJECTIONS_MAX - 1
        );
        assert_eq!(
            projections
                .last()
                .map(|projection| projection.root_event_id.as_str()),
            Some(expected_last.as_str())
        );
    }

    #[test]
    fn replay_known_root_is_not_suppressed_by_an_older_canonical_reply() {
        let mut root = timeline_item("$known-root:test", Some("root"), "@alice:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 2,
            latest_event_id: Some("$latest-summary-reply:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut older_reply = timeline_item("$older-reply:test", Some("older"), "@b:test", false);
        older_reply.timestamp_ms = Some(300);
        older_reply.thread_root = Some("$known-root:test".to_owned());
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);

        let projections =
            known_thread_root_projections_for_replay(&[root], &[before, older_reply, after]);

        assert!(matches!(
            projections.as_slice(),
            [ThreadRootProjectionDto {
                root_event_id,
                activity_event_id,
                activity_timestamp_ms: Some(400),
                retain_without_reply: true,
                state: ThreadRootProjectionStateDto::Ready { .. },
                ..
            }]
                if root_event_id == "$known-root:test"
                    && activity_event_id == "$latest-summary-reply:test"
        ));
    }

    #[test]
    fn replay_known_registry_reconciles_diff_removals_and_all_initial_refreshes() {
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@alice:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$summary-activity:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let navigation = vec![root];
        let display = vec![before, after];

        for initial_refresh in [
            "actor_spawn",
            "send_queue_lag",
            "queue_overflow",
            "sync_started_replacement",
        ] {
            let registry = Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            ));
            let initial =
                refresh_replay_known_root_projections(&registry, &key, &navigation, &display);
            assert!(matches!(
                initial.ready.as_slice(),
                [ThreadRootProjectionDto {
                    retain_without_reply: true,
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                }]
            ));
            let refreshed = refresh_replay_known_root_projections(&registry, &key, &[], &display);
            assert!(
                refreshed.ready.is_empty(),
                "{initial_refresh} must not retain an absent root"
            );
            assert!(matches!(
                refreshed.stale.as_slice(),
                [ReplayKnownThreadRootProjection { root_event_id, .. }]
                    if root_event_id == "$known-root:test"
            ));
        }

        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let _ = refresh_replay_known_root_projections(&registry, &key, &navigation, &display);
        let stale_after_diff = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &[],
            &ReplayKnownDisplayContext::from_display_items(&display),
        );
        assert!(matches!(
            stale_after_diff.stale.as_slice(),
            [ReplayKnownThreadRootProjection { root_event_id, .. }]
                if root_event_id == "$known-root:test"
        ));
        assert!(stale_after_diff.ready.is_empty());
    }

    #[tokio::test]
    async fn replay_known_navigation_summary_change_replaces_the_ready_snapshot() {
        let key = room_key();
        let root_with_summary = |activity_event_id: &str, activity_timestamp_ms: u64| {
            let mut root = timeline_item("$known-root:test", Some("root"), "@alice:test", false);
            root.thread_summary = Some(ThreadSummaryDto {
                reply_count: 2,
                latest_event_id: Some(activity_event_id.to_owned()),
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(activity_timestamp_ms),
            });
            root
        };
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(600);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;

        let initial = {
            let _lease = actor_generations
                .try_acquire(&key, actor_generation)
                .expect("current actor must acquire a replay-known lease");
            refresh_replay_known_root_projections(
                &registry,
                &key,
                &[root_with_summary("$old-summary:test", 300)],
                &[before.clone(), after.clone()],
            )
        };
        assert!(matches!(
            initial.ready.as_slice(),
            [ThreadRootProjectionDto {
                activity_event_id,
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                ..
            }] if activity_event_id == "$old-summary:test"
        ));

        let replacement = {
            let _lease = actor_generations
                .try_acquire(&key, actor_generation)
                .expect("current actor must acquire a replay-known lease");
            reconcile_replay_known_root_projections_after_navigation_update(
                &registry,
                &key,
                &[root_with_summary("$new-summary:test", 500)],
                &ReplayKnownDisplayContext::from_display_items(&[before.clone(), after.clone()]),
            )
        };

        assert!(matches!(
            replacement.stale.as_slice(),
            [ReplayKnownThreadRootProjection {
                activity_event_id,
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                ..
            }] if activity_event_id == "$old-summary:test"
        ));
        assert!(matches!(
            replacement.ready.as_slice(),
            [ThreadRootProjectionDto {
                root_event_id,
                activity_event_id,
                activity_timestamp_ms: Some(500),
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 2 },
                state: ThreadRootProjectionStateDto::Ready { .. },
            }] if root_event_id == "$known-root:test" && activity_event_id == "$new-summary:test"
        ));

        let (event_tx, mut event_rx) = broadcast::channel(8);
        emit_replay_known_root_projection_update(&event_tx, &key, replacement);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 2 },
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            }))
        ));
    }

    #[test]
    fn replay_known_same_activity_reemits_a_renderable_root_revision_without_a_clear() {
        let key = room_key();
        let root_with_summary = |body: &str| {
            let mut root = timeline_item("$known-root:test", Some(body), "@alice:test", false);
            root.thread_summary = Some(ThreadSummaryDto {
                reply_count: 2,
                latest_event_id: Some("$latest:test".to_owned()),
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(400),
            });
            root
        };
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let display_context =
            ReplayKnownDisplayContext::from_display_items(&[before.clone(), after.clone()]);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let initial = refresh_replay_known_root_projections(
            &registry,
            &key,
            &[root_with_summary("original")],
            &[before.clone(), after.clone()],
        );
        assert!(matches!(
            initial.ready.as_slice(),
            [ThreadRootProjectionDto {
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                state: ThreadRootProjectionStateDto::Ready { item },
                ..
            }] if item.body.as_deref() == Some("original")
        ));

        let mut revised_root = root_with_summary("redacted replacement");
        revised_root.is_redacted = true;
        revised_root.reactions = vec![ReactionGroup {
            key: "👍".to_owned(),
            count: 2,
            reacted_by_me: true,
            my_reaction_event_id: Some("$reaction:test".to_owned()),
            sender_preview: vec![
                ReactionSender {
                    user_id: "@alice:test".to_owned(),
                    display_label: Some("Alice".to_owned()),
                },
                ReactionSender {
                    user_id: "@bob:test".to_owned(),
                    display_label: Some("Bob".to_owned()),
                },
            ],
        }];
        revised_root.actions = TimelineMessageActions {
            can_copy: false,
            can_forward: false,
            can_reply: false,
            can_permalink: true,
            can_view_source: true,
            permalink: Some("https://example.invalid/#/room/$known-root:test".to_owned()),
            editable_document: None,
        };
        let update = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &[revised_root.clone()],
            &display_context,
        );

        assert!(update.stale.is_empty());
        assert!(matches!(
            update.ready.as_slice(),
            [ThreadRootProjectionDto {
                activity_event_id,
                activity_timestamp_ms: Some(400),
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                state: ThreadRootProjectionStateDto::Ready { item },
                ..
            }]
                if activity_event_id == "$latest:test"
                    && item == &revised_root
        ));

        let (event_tx, mut event_rx) = broadcast::channel(8);
        emit_replay_known_root_projection_update(&event_tx, &key, update);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                key: emitted_key,
                projection: ThreadRootProjectionDto {
                    activity_event_id,
                    activity_timestamp_ms: Some(400),
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    state: ThreadRootProjectionStateDto::Ready { item },
                    ..
                },
            }))
                if emitted_key == key
                    && activity_event_id == "$latest:test"
                    && item == revised_root
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let unchanged = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &[revised_root],
            &display_context,
        );
        assert!(unchanged.stale.is_empty());
        assert!(unchanged.ready.is_empty());
    }

    #[tokio::test]
    async fn non_sdk_ignored_root_set_revises_replay_ready_without_corrupting_bounded_display() {
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@ignored:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$latest:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);

        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let initial = refresh_replay_known_root_projections(
            &registry,
            &key,
            &[root.clone()],
            &[before.clone(), after.clone()],
        );
        assert!(matches!(
            initial.ready.as_slice(),
            [ThreadRootProjectionDto {
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                state: ThreadRootProjectionStateDto::Ready { item },
                ..
            }] if !item.is_hidden
        ));

        let mut ignored_root = root.clone();
        apply_ignored_sender_suppression(
            &mut ignored_root,
            &std::collections::BTreeSet::from(["@ignored:test".to_owned()]),
        );
        let diffs = vec![TimelineDiff::Set {
            index: 0,
            item: ignored_root.clone(),
        }];
        let navigation_items = vec![ignored_root.clone(), before.clone(), after.clone()];
        let mut display_projection = DisplayProjectionState::from_canonical_window(
            &navigation_items,
            1..navigation_items.len(),
        );
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (event_tx, mut event_rx) = broadcast::channel(8);

        assert!(
            emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &service,
                &generations,
                &key,
                actor_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                diffs.clone(),
                &navigation_items,
                &mut display_projection,
            )
        );

        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs: emitted, .. }))
                if emitted.is_empty()
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    state: ThreadRootProjectionStateDto::Ready { item },
                    ..
                },
                ..
            })) if item == ignored_root && item.is_hidden
        ));
        assert_eq!(display_projection.display_items(), &[before, after]);
        assert!(
            registry
                .lock()
                .expect("registry lock")
                .get(&key)
                .and_then(|entries| entries.get("$known-root:test"))
                .is_some_and(|entry| entry.item.is_hidden),
            "the replay registry must keep the revised snapshot, not the stale visible one"
        );
    }

    #[tokio::test]
    async fn non_sdk_link_preview_set_reemits_same_epoch_replay_ready_revision() {
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$latest:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let initial = refresh_replay_known_root_projections(
            &registry,
            &key,
            &[root.clone()],
            &[before.clone(), after.clone()],
        );
        assert!(matches!(
            initial.ready.as_slice(),
            [ThreadRootProjectionDto {
                source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                ..
            }]
        ));

        let mut revised_root = root.clone();
        revised_root.link_previews = Some(vec![LinkPreview {
            url: "https://example.test/preview".to_owned(),
            title: Some("ready preview".to_owned()),
            description: None,
            image: None,
            state: LinkPreviewState::Ready,
        }]);
        let diffs = vec![TimelineDiff::Set {
            index: 0,
            item: revised_root.clone(),
        }];
        let navigation_items = vec![revised_root.clone(), before, after];
        let mut display_projection = DisplayProjectionState::from_canonical_window(
            &navigation_items,
            1..navigation_items.len(),
        );
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (event_tx, mut event_rx) = broadcast::channel(8);

        assert!(
            emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &service,
                &generations,
                &key,
                actor_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                diffs,
                &navigation_items,
                &mut display_projection,
            )
        );

        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }))
                if diffs.is_empty()
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    state: ThreadRootProjectionStateDto::Ready { item },
                    ..
                },
                ..
            })) if item == revised_root
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn non_sdk_set_updates_an_ordinary_bounded_display_item_by_canonical_owner() {
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$latest:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut ordinary = timeline_item("$ordinary:test", Some("before"), "@a:test", false);
        ordinary.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let _ = refresh_replay_known_root_projections(
            &registry,
            &key,
            &[root.clone(), ordinary.clone(), after.clone()],
            &[ordinary.clone(), after.clone()],
        );

        let mut revised_ordinary = ordinary.clone();
        revised_ordinary.body = Some("after".to_owned());
        let diffs = vec![TimelineDiff::Set {
            index: 1,
            item: revised_ordinary.clone(),
        }];
        let canonical_before = vec![root.clone(), ordinary, after.clone()];
        let mut display_projection = DisplayProjectionState::from_canonical_window(
            &canonical_before,
            1..canonical_before.len(),
        );
        let navigation_items = vec![root, revised_ordinary.clone(), after.clone()];
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (event_tx, mut event_rx) = broadcast::channel(8);

        assert!(
            emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &service,
                &generations,
                &key,
                actor_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                diffs.clone(),
                &navigation_items,
                &mut display_projection,
            )
        );

        assert_eq!(
            display_projection.display_items(),
            &[revised_ordinary.clone(), after]
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs: emitted, .. }))
                if emitted
                    == vec![TimelineDiff::Set {
                        index: 0,
                        item: revised_ordinary.clone(),
                    }]
        ));
        assert!(
            matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "an unchanged replay root must not be re-emitted for an ordinary row mutation"
        );
    }

    #[tokio::test]
    async fn stale_non_sdk_set_cannot_mutate_display_mirror_registry_or_emit_events() {
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$latest:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let _ = refresh_replay_known_root_projections(
            &registry,
            &key,
            &[root.clone()],
            &[before.clone(), after.clone()],
        );
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let stale_generation = generations.activate_after_quiescence(&key).await.generation;
        let _current_generation = generations.activate_after_quiescence(&key).await.generation;
        let mut stale_root = root.clone();
        stale_root.is_hidden = true;
        let display_items = vec![before.clone(), after.clone()];
        let mut display_projection =
            DisplayProjectionState::from_canonical_window(&display_items, 0..display_items.len());
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let (event_tx, mut event_rx) = broadcast::channel(8);

        assert!(
            !emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &service,
                &generations,
                &key,
                stale_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                vec![TimelineDiff::Set {
                    index: 0,
                    item: stale_root,
                }],
                &[root],
                &mut display_projection,
            )
        );
        assert_eq!(display_projection.display_items(), &[before, after]);
        assert!(
            registry
                .lock()
                .expect("registry lock")
                .get(&key)
                .and_then(|entries| entries.get("$known-root:test"))
                .is_some_and(|entry| !entry.item.is_hidden)
        );
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn non_sdk_item_sets_update_the_exact_canonical_duplicate_owner() {
        let first_owner = timeline_item("$duplicate:test", Some("first"), "@a:test", false);
        let second_owner = timeline_item("$duplicate:test", Some("second"), "@a:test", false);
        let replacement = timeline_item("$replacement:test", Some("replacement"), "@a:test", false);
        let canonical = vec![first_owner.clone(), second_owner];
        let mut display_projection =
            DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
        let projected = apply_non_sdk_item_set_diffs_to_display_items(
            &mut display_projection,
            &[TimelineDiff::Set {
                index: 1,
                item: replacement.clone(),
            }],
        );

        assert_eq!(
            display_projection.display_items(),
            &[first_owner, replacement.clone()],
            "the Set index must select the second canonical owner, not the first matching identity"
        );
        assert_eq!(
            projected,
            vec![TimelineDiff::Insert {
                index: 1,
                item: replacement,
            }]
        );
    }

    #[test]
    fn non_sdk_item_sets_ignore_exact_owners_outside_the_display_window() {
        let visible = timeline_item("$visible:test", Some("visible"), "@a:test", false);
        let hidden_duplicate = timeline_item("$duplicate:test", Some("hidden"), "@a:test", false);
        let replacement = timeline_item("$replacement:test", Some("replacement"), "@a:test", false);
        let canonical = vec![visible.clone(), hidden_duplicate];
        let mut display_projection =
            DisplayProjectionState::from_canonical_window(&canonical, 0..1);
        let projected = apply_non_sdk_item_set_diffs_to_display_items(
            &mut display_projection,
            &[TimelineDiff::Set {
                index: 1,
                item: replacement,
            }],
        );

        assert_eq!(display_projection.display_items(), &[visible]);
        assert!(projected.is_empty());
    }

    #[tokio::test]
    async fn stale_actor_generation_cannot_clear_new_replay_known_registry_state() {
        let key = room_key();
        let mut root = timeline_item("$known-root:test", Some("root"), "@alice:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$summary:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(400),
        });
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let old_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let initial = {
            let _lease = actor_generations
                .try_acquire(&key, old_generation)
                .expect("old actor must initially own the gate");
            refresh_replay_known_root_projections(
                &registry,
                &key,
                &[root.clone()],
                &[before.clone(), after.clone()],
            )
        };
        assert_eq!(initial.ready.len(), 1);

        // Model an old actor that has begun its registry/Core section when
        // SyncStarted arrives. Replacement must mark it stale immediately,
        // wait for this short lease, and only then activate the new actor.
        let old_lease = actor_generations
            .try_acquire(&key, old_generation)
            .expect("old actor lease");
        let gate_for_replacement = actor_generations.clone();
        let key_for_replacement = key.clone();
        let replacement = tokio::spawn(async move {
            gate_for_replacement
                .activate_after_quiescence(&key_for_replacement)
                .await
        });
        let mut replacement_has_fenced_old_actor = false;
        for _ in 0..10 {
            if actor_generations
                .try_acquire(&key, old_generation)
                .is_none()
            {
                replacement_has_fenced_old_actor = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            replacement_has_fenced_old_actor,
            "replacement must stop an old delayed actor before waiting for its in-flight lease"
        );
        assert!(
            !replacement.is_finished(),
            "replacement must wait until the in-flight synchronous lease quiesces"
        );
        drop(old_lease);
        let new_generation = replacement
            .await
            .expect("replacement generation task")
            .generation;
        assert_eq!(
            actor_generations.current_generation(&key),
            Some(new_generation),
            "the quiesced replacement must publish the new generation before its actor refreshes"
        );

        let stale_update = actor_generations
            .try_acquire(&key, old_generation)
            .map(|_lease| {
                reconcile_replay_known_root_projections_after_navigation_update(
                    &registry,
                    &key,
                    &[],
                    &ReplayKnownDisplayContext::from_display_items(&[
                        before.clone(),
                        after.clone(),
                    ]),
                )
            })
            .unwrap_or_default();
        assert!(stale_update.ready.is_empty());
        assert!(stale_update.stale.is_empty());
        assert!(
            registry
                .lock()
                .expect("registry lock")
                .get(&key)
                .is_some_and(|entries| entries.contains_key("$known-root:test"))
        );

        let (event_tx, mut event_rx) = broadcast::channel(8);
        emit_replay_known_root_projection_update(&event_tx, &key, stale_update);
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let new_actor_ready_count = {
            let _lease = actor_generations
                .try_acquire(&key, new_generation)
                .expect("new actor must own the replacement generation");
            let update =
                refresh_replay_known_root_projections(&registry, &key, &[root], &[before, after]);
            let ready_count = update.ready.len();
            emit_replay_known_root_projection_update(&event_tx, &key, update);
            ready_count
        };
        assert_eq!(new_actor_ready_count, 1);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            }))
        ));
    }

    #[test]
    fn replay_known_reconciliation_uses_the_bounded_display_context_not_cache_replies() {
        let key = room_key();
        let root_with_summary = |activity_event_id: &str, activity_timestamp_ms: u64| {
            let mut root = timeline_item("$known-root:test", Some("root"), "@alice:test", false);
            root.thread_summary = Some(ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some(activity_event_id.to_owned()),
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(activity_timestamp_ms),
            });
            root
        };
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(500);
        let mut cache_only_latest_reply =
            timeline_item("$latest:test", Some("reply"), "@b:test", false);
        cache_only_latest_reply.timestamp_ms = Some(400);
        cache_only_latest_reply.thread_root = Some("$known-root:test".to_owned());
        let mut displayed_items = vec![before.clone(), after];
        let display_context = ReplayKnownDisplayContext::from_display_items(&displayed_items);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let navigation = vec![
            root_with_summary("$latest:test", 400),
            cache_only_latest_reply.clone(),
        ];

        let initial = refresh_replay_known_root_projections_with_display_context(
            &registry,
            &key,
            &navigation,
            &display_context,
        );
        assert_eq!(
            initial.ready.len(),
            1,
            "cache-only reply is not display evidence"
        );

        let unrelated = timeline_item("$unrelated:test", Some("other"), "@c:test", false);
        apply_timeline_diffs_to_items(
            &mut displayed_items,
            &[TimelineDiff::PushBack {
                item: unrelated.clone(),
            }],
        );
        let unchanged = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &[navigation.clone(), vec![unrelated]].concat(),
            &ReplayKnownDisplayContext::from_display_items(&displayed_items),
        );
        assert!(unchanged.stale.is_empty());
        assert!(unchanged.ready.is_empty());

        let outside_range = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &[
                root_with_summary("$later:test", 600),
                cache_only_latest_reply.clone(),
            ],
            &display_context,
        );
        assert!(matches!(
            outside_range.stale.as_slice(),
            [ReplayKnownThreadRootProjection { root_event_id, .. }]
                if root_event_id == "$known-root:test"
        ));
        assert!(outside_range.ready.is_empty());

        let _ = refresh_replay_known_root_projections_with_display_context(
            &registry,
            &key,
            &navigation,
            &display_context,
        );
        apply_timeline_diffs_to_items(
            &mut displayed_items,
            &[TimelineDiff::PushBack {
                item: cache_only_latest_reply,
            }],
        );
        let displayed_latest_reply =
            ReplayKnownDisplayContext::from_display_items(&displayed_items);
        let exact_reply = reconcile_replay_known_root_projections_after_navigation_update(
            &registry,
            &key,
            &navigation,
            &displayed_latest_reply,
        );
        assert!(matches!(
            exact_reply.stale.as_slice(),
            [ReplayKnownThreadRootProjection { root_event_id, .. }]
                if root_event_id == "$known-root:test"
        ));
        assert!(exact_reply.ready.is_empty());
    }

    #[tokio::test]
    async fn hydration_preparation_replaced_during_capacity_wait_publishes_nothing() {
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let old_generation = generations.activate_after_quiescence(&key).await.generation;
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$stale-root:test".to_owned(),
            activity_event_id: "$stale-reply:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let mut second_activity = activity.clone();
        second_activity.root_event_id = "$second-stale-root:test".to_owned();
        second_activity.activity_event_id = "$second-stale-reply:test".to_owned();
        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .send(vec![AppAction::ThreadRootProjectionsCleared {
                room_id: "!occupied:test".to_owned(),
            }])
            .await
            .expect("fill reducer channel");
        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let commit = tokio::spawn({
            let service = Arc::clone(&service);
            let replay_registry = Arc::clone(&replay_registry);
            let generations = Arc::clone(&generations);
            let action_tx = action_tx.clone();
            let manager_tx = manager_tx.clone();
            let event_tx = event_tx.clone();
            let key = key.clone();
            let activity = activity.clone();
            let second_activity = second_activity.clone();
            async move {
                commit_prepared_thread_root_hydration_for_generation(
                    &service,
                    &replay_registry,
                    &generations,
                    &action_tx,
                    &manager_tx,
                    &event_tx,
                    &key,
                    old_generation,
                    None,
                    PreparedThreadRootHydration {
                        activities_by_root: HashMap::from([
                            (activity.root_event_id.clone(), activity.clone()),
                            (
                                second_activity.root_event_id.clone(),
                                second_activity.clone(),
                            ),
                        ]),
                        // More fetches than manager mailbox capacity must still
                        // reserve one atomic batch rather than deadlocking on
                        // unpublished per-fetch permits.
                        missing_activities: vec![activity, second_activity],
                        canonical_root_event_ids: HashSet::new(),
                        redacted_activity_event_ids: HashSet::new(),
                        refresh_root_event_ids: None,
                    },
                )
                .await
            }
        });
        tokio::task::yield_now().await;
        let replacement = generations.activate_after_quiescence(&key).await.generation;
        assert_ne!(replacement, old_generation);

        let _occupied = action_rx.recv().await.expect("occupied reducer action");
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), commit)
                .await
                .expect("batched fetch reservation must not deadlock")
                .expect("stale hydration commit")
        );
        assert!(
            !service
                .lock()
                .expect("service lock")
                .has_pending_attempt(&activity)
        );
        assert!(action_rx.try_recv().is_err(), "no stale projection action");
        assert!(manager_rx.try_recv().is_err(), "no stale manager fetch");
        assert!(event_rx.try_recv().is_err(), "no stale projection event");
    }

    #[tokio::test]
    async fn hydration_does_not_hold_action_capacity_while_waiting_for_manager_capacity() {
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .send(vec![AppAction::ThreadRootProjectionsCleared {
                room_id: "!occupied:test".to_owned(),
            }])
            .await
            .expect("fill reducer channel");
        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        manager_tx
            .send(TimelineMessage::IgnoredUsersUpdated {
                user_ids: std::collections::BTreeSet::new(),
            })
            .await
            .expect("fill manager mailbox");
        let earlier_manager_tx = manager_tx.clone();
        let mut earlier_manager_sender = Box::pin(earlier_manager_tx.send(
            TimelineMessage::IgnoredUsersUpdated {
                user_ids: std::collections::BTreeSet::new(),
            },
        ));
        assert_pending_on_first_poll(earlier_manager_sender.as_mut(), "earlier manager sender")
            .await;

        let (event_tx, _) = broadcast::channel(8);
        let mut hydration = Box::pin(commit_prepared_thread_root_hydration_for_generation(
            &service,
            &replay_registry,
            &generations,
            &action_tx,
            &manager_tx,
            &event_tx,
            &key,
            actor_generation,
            None,
            PreparedThreadRootHydration {
                activities_by_root: HashMap::from([(
                    activity.root_event_id.clone(),
                    activity.clone(),
                )]),
                missing_activities: vec![activity.clone()],
                canonical_root_event_ids: HashSet::new(),
                redacted_activity_event_ids: HashSet::new(),
                refresh_root_event_ids: None,
            },
        ));
        assert_pending_on_first_poll(hydration.as_mut(), "hydration reservation").await;

        let _initial_manager_message = manager_rx.recv().await.expect("manager message");
        earlier_manager_sender.await.expect("manager mailbox open");
        let manager_action_tx = action_tx.clone();
        let mut manager_action = Box::pin(
            manager_action_tx.send(vec![AppAction::ActivityRowsObserved { rows: Vec::new() }]),
        );
        assert_pending_on_first_poll(manager_action.as_mut(), "manager reducer action").await;
        let _occupied_action = action_rx.recv().await.expect("occupied reducer action");
        let manager_action_poll = poll_fn(|cx| Poll::Ready(manager_action.as_mut().poll(cx))).await;
        assert!(
            matches!(manager_action_poll, Poll::Ready(Ok(()))),
            "manager must own the first freed reducer slot"
        );

        let _earlier_manager_message = manager_rx.recv().await.expect("earlier manager message");
        assert_pending_on_first_poll(hydration.as_mut(), "hydration reducer reservation").await;
        let manager_reducer_action = action_rx.recv().await.expect("manager reducer action");
        assert!(matches!(
            manager_reducer_action.as_slice(),
            [AppAction::ActivityRowsObserved { .. }]
        ));
        assert!(hydration.await);
        let hydration_action = action_rx.recv().await.expect("hydration reducer action");
        assert!(matches!(
            hydration_action.first(),
            Some(AppAction::ThreadRootProjectionsReconciled { .. })
        ));
        assert!(matches!(
            manager_rx.recv().await,
            Some(TimelineMessage::StartAggregateRefresh {
                actor_generation: generation,
                refreshes,
                ..
            }) if generation == actor_generation
                && refreshes.len() == 1
                && refreshes[0].activity == activity
                && refreshes[0].cause == AggregateRefreshCause::InitialHydration
                && refreshes[0].root_active
                && refreshes[0].hydrate_root
        ));
        assert!(
            manager_rx.try_recv().is_err(),
            "hydration is not duplicated"
        );
        assert!(
            service
                .lock()
                .expect("service lock")
                .has_pending_attempt(&activity)
        );
    }

    #[test]
    fn thread_and_focused_items_do_not_claim_room_canonical_summary_ownership() {
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let mut root = timeline_item("$root:test", Some("root"), "@root:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$reply:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: Some("reply".to_owned()),
            latest_timestamp_ms: Some(100),
        });
        super::seed_thread_summary_item(&service, &thread_key(), &root);
        assert!(
            service
                .lock()
                .expect("service lock")
                .current_aggregate("!r:test", "$root:test")
                .is_none()
        );
    }

    #[test]
    fn room_batches_refresh_only_roots_with_changed_root_or_reply_items() {
        let key = room_key();
        let mut root = timeline_item("$root:test", Some("root"), "@root:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$reply-a:test".to_owned()),
            latest_sender: Some("@a:test".to_owned()),
            latest_sender_label: Some("A".to_owned()),
            latest_body_preview: Some("A".to_owned()),
            latest_timestamp_ms: Some(100),
        });
        let mut reply_a = timeline_item("$reply-a:test", Some("A"), "@a:test", false);
        reply_a.thread_root = Some("$root:test".to_owned());
        reply_a.timestamp_ms = Some(100);
        let unrelated_before = timeline_item("$other:test", Some("before"), "@o:test", false);
        let unrelated_after = timeline_item("$other:test", Some("after"), "@o:test", false);
        assert!(
            thread_summary_affected_root_event_ids(
                &key,
                &[root.clone(), reply_a.clone(), unrelated_before],
                &[root.clone(), reply_a.clone(), unrelated_after],
            )
            .is_empty(),
            "an unrelated message Set must not query every thread aggregate"
        );

        let mut reply_b = timeline_item("$reply-b:test", Some("B"), "@b:test", false);
        reply_b.thread_root = Some("$root:test".to_owned());
        reply_b.timestamp_ms = Some(200);
        assert_eq!(
            thread_summary_affected_root_event_ids(
                &key,
                &[root.clone(), reply_a.clone()],
                &[root.clone(), reply_a, reply_b.clone()],
            ),
            HashSet::from(["$root:test".to_owned()])
        );
        reply_b.is_redacted = true;
        assert_eq!(
            thread_summary_affected_root_event_ids(
                &key,
                &[
                    root.clone(),
                    thread_reply_item("$reply-b:test", "@b:test", "$root:test")
                ],
                &[root, reply_b],
            ),
            HashSet::from(["$root:test".to_owned()])
        );
    }

    #[tokio::test]
    async fn unrelated_room_batch_does_not_refresh_a_canonical_root_without_reply_row() {
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        service
            .lock()
            .expect("service lock")
            .seed_canonical_summary(
                key.room_id(),
                "$root:test",
                &ThreadSummaryDto {
                    reply_count: 1,
                    latest_event_id: Some("$reply:test".to_owned()),
                    latest_sender: None,
                    latest_sender_label: None,
                    latest_body_preview: Some("reply".to_owned()),
                    latest_timestamp_ms: Some(100),
                },
            );
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let (action_tx, mut action_rx) = mpsc::channel(2);
        let (manager_tx, mut manager_rx) = mpsc::channel(2);
        let (event_tx, _) = broadcast::channel(8);

        assert!(
            commit_prepared_thread_root_hydration_for_generation(
                &service,
                &replay_registry,
                &generations,
                &action_tx,
                &manager_tx,
                &event_tx,
                &key,
                actor_generation,
                None,
                PreparedThreadRootHydration {
                    activities_by_root: HashMap::new(),
                    missing_activities: Vec::new(),
                    canonical_root_event_ids: HashSet::from(["$root:test".to_owned()]),
                    redacted_activity_event_ids: HashSet::new(),
                    refresh_root_event_ids: Some(HashSet::new()),
                },
            )
            .await
        );
        assert!(matches!(
            action_rx.recv().await,
            Some(actions) if matches!(
                actions.as_slice(),
                [AppAction::ThreadRootProjectionsReconciled { .. }]
            )
        ));
        assert!(
            manager_rx.try_recv().is_err(),
            "an unrelated batch must not abort and respawn aggregate work"
        );
    }

    #[tokio::test]
    async fn canonical_root_with_live_reply_schedules_authoritative_summary_refresh() {
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply-b:test".to_owned(),
            activity_timestamp_ms: Some(200),
            activity_sender: Some("@b:test".to_owned()),
            activity_sender_label: Some("B".to_owned()),
            activity_body_preview: Some("new reply".to_owned()),
        };
        let (action_tx, mut action_rx) = mpsc::channel(2);
        let (manager_tx, mut manager_rx) = mpsc::channel(2);
        let (event_tx, _) = broadcast::channel(8);

        assert!(
            commit_prepared_thread_root_hydration_for_generation(
                &service,
                &replay_registry,
                &generations,
                &action_tx,
                &manager_tx,
                &event_tx,
                &key,
                actor_generation,
                None,
                PreparedThreadRootHydration {
                    activities_by_root: HashMap::from([(
                        activity.root_event_id.clone(),
                        activity.clone(),
                    )]),
                    // The root is already canonical, so no root hydration fetch
                    // is needed; its live reply still requires an aggregate refresh.
                    missing_activities: Vec::new(),
                    canonical_root_event_ids: HashSet::from([activity.root_event_id.clone()]),
                    redacted_activity_event_ids: HashSet::new(),
                    refresh_root_event_ids: None,
                },
            )
            .await
        );
        assert!(matches!(
            action_rx.recv().await,
            Some(actions) if matches!(
                actions.as_slice(),
                [AppAction::ThreadRootProjectionsReconciled { .. }]
            )
        ));
        assert!(matches!(
            manager_rx.try_recv(),
            Ok(TimelineMessage::StartAggregateRefresh {
                actor_generation: generation,
                refreshes,
                ..
            }) if generation == actor_generation
                && refreshes.len() == 1
                && refreshes[0].activity == activity
                && refreshes[0].root_active
                && !refreshes[0].hydrate_root
        ));
    }

    #[test]
    fn newer_sdk_summary_is_detected_before_overlay_and_repaired_by_exact_aggregate() {
        let key = room_key();
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let mut root_a = timeline_item("$root:test", Some("root"), "@root:test", false);
        root_a.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$reply-a:test".to_owned()),
            latest_sender: Some("@a:test".to_owned()),
            latest_sender_label: Some("A".to_owned()),
            latest_body_preview: Some("A".to_owned()),
            latest_timestamp_ms: Some(100),
        });
        super::seed_thread_summary_item(&service, &key, &root_a);

        let mut root_b = root_a.clone();
        root_b.thread_summary = Some(ThreadSummaryDto {
            reply_count: 2,
            latest_event_id: Some("$reply-b:test".to_owned()),
            latest_sender: Some("@b:test".to_owned()),
            latest_sender_label: Some("B".to_owned()),
            latest_body_preview: Some("B".to_owned()),
            latest_timestamp_ms: Some(200),
        });
        let raw_diff = TimelineDiff::Set {
            index: 0,
            item: root_b.clone(),
        };
        let mut raw_after = vec![root_a.clone()];
        apply_timeline_diffs_to_items(&mut raw_after, std::slice::from_ref(&raw_diff));
        assert_eq!(
            thread_summary_affected_root_event_ids(&key, &[root_a.clone()], &raw_after),
            HashSet::from(["$root:test".to_owned()])
        );

        // The bundled identity is provisional (it may be an edit event), so
        // overlay retains A until the exact event-cache aggregate validates B.
        super::seed_thread_summary_diff(&service, &key, &raw_diff);
        let mut overlaid_diff = raw_diff;
        overlay_thread_summary_diff(&service, &key, &mut overlaid_diff);
        let TimelineDiff::Set { item, .. } = &overlaid_diff else {
            panic!("expected root Set")
        };
        assert_eq!(
            item.thread_summary
                .as_ref()
                .and_then(|summary| summary.latest_event_id.as_deref()),
            Some("$reply-a:test")
        );

        let activity = service
            .lock()
            .expect("service lock")
            .activity_for_root(key.room_id(), "$root:test")
            .expect("tracked root");
        let refresh = service
            .lock()
            .expect("service lock")
            .schedule_aggregate_refresh_with_canonical_root(
                &activity,
                AggregateRefreshCause::CanonicalBatch,
                true,
                true,
                false,
            )
            .expect("aggregate refresh");
        assert!(matches!(
            service.lock().expect("service lock").complete_refresh(
                &refresh,
                Ok(
                    crate::threads_list::ThreadRootProjectionRefreshResult::Aggregate(
                        AuthoritativeThreadAggregate {
                            reply_count: 2,
                            latest_event_id: Some("$reply-b:test".to_owned()),
                            latest_sender: Some("@b:test".to_owned()),
                            latest_sender_label: Some("B".to_owned()),
                            latest_body_preview: Some("B".to_owned()),
                            latest_timestamp_ms: Some(200),
                        },
                    )
                ),
            ),
            crate::threads_list::ThreadRootProjectionCompletion::Updated(_)
        ));
        let mut validated_diff = TimelineDiff::Set {
            index: 0,
            item: root_b,
        };
        overlay_thread_summary_diff(&service, &key, &mut validated_diff);
        let TimelineDiff::Set { item, .. } = validated_diff else {
            panic!("expected validated root Set")
        };
        assert_eq!(
            item.thread_summary
                .as_ref()
                .and_then(|summary| summary.latest_event_id.as_deref()),
            Some("$reply-b:test")
        );
        assert_eq!(
            item.thread_summary
                .as_ref()
                .map(|summary| summary.reply_count),
            Some(2)
        );
    }

    #[tokio::test]
    async fn accepted_canonical_aggregate_emits_an_authoritative_set_and_overlays_stale_sdk_input()
    {
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply-b:test".to_owned(),
            activity_timestamp_ms: Some(200),
            activity_sender: Some("@b:test".to_owned()),
            activity_sender_label: Some("B".to_owned()),
            activity_body_preview: Some("B".to_owned()),
        };
        let mut root = timeline_item("$root:test", Some("root"), "@root:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$reply-a:test".to_owned()),
            latest_sender: Some("@a:test".to_owned()),
            latest_sender_label: Some("A".to_owned()),
            latest_body_preview: Some("A".to_owned()),
            latest_timestamp_ms: Some(100),
        });
        let aggregate_b = AuthoritativeThreadAggregate {
            reply_count: 2,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: activity.activity_sender.clone(),
            latest_sender_label: activity.activity_sender_label.clone(),
            latest_body_preview: activity.activity_body_preview.clone(),
            latest_timestamp_ms: activity.activity_timestamp_ms,
        };
        {
            let mut service_guard = service.lock().expect("service lock");
            assert!(matches!(
                service_guard.observe(activity.clone()),
                ThreadRootProjectionDecision::StartFetch(_)
            ));
            service_guard.set_canonical_root_event_ids(
                key.room_id(),
                &HashSet::from([activity.root_event_id.clone()]),
            );
            let refresh = service_guard
                .schedule_aggregate_refresh_with_canonical_root(
                    &activity,
                    AggregateRefreshCause::SelectedActivity,
                    true,
                    true,
                    false,
                )
                .expect("canonical aggregate refresh");
            assert!(matches!(
                service_guard.complete_refresh(
                    &refresh,
                    Ok(
                        crate::threads_list::ThreadRootProjectionRefreshResult::Aggregate(
                            aggregate_b.clone(),
                        )
                    ),
                ),
                crate::threads_list::ThreadRootProjectionCompletion::Updated(_)
            ));
        }

        let mut stale_sdk_set = TimelineDiff::Set {
            index: 0,
            item: root.clone(),
        };
        overlay_thread_summary_diff(&service, &key, &mut stale_sdk_set);
        let TimelineDiff::Set { item, .. } = &stale_sdk_set else {
            panic!("expected a Set diff");
        };
        assert_eq!(
            item.thread_summary.as_ref().unwrap().latest_event_id,
            aggregate_b.latest_event_id
        );
        assert_eq!(
            item.thread_summary.as_ref().unwrap().latest_body_preview,
            aggregate_b.latest_body_preview
        );

        let (event_tx, mut event_rx) = broadcast::channel(8);
        let mut display_projection =
            DisplayProjectionState::from_canonical_window(std::slice::from_ref(&root), 0..1);
        assert!(
            emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
                &event_tx,
                &replay_registry,
                &service,
                &generations,
                &key,
                actor_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                vec![stale_sdk_set],
                std::slice::from_ref(&root),
                &mut display_projection,
            )
        );
        let event = event_rx.recv().await.expect("canonical Set event");
        let CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) = event else {
            panic!("expected canonical ItemsUpdated event");
        };
        let [TimelineDiff::Set { item, .. }] = diffs.as_slice() else {
            panic!("expected one canonical Set diff");
        };
        assert_eq!(item.thread_summary.as_ref().unwrap().reply_count, 2);
        assert_eq!(
            item.thread_summary
                .as_ref()
                .unwrap()
                .latest_event_id
                .as_deref(),
            Some("$reply-b:test")
        );
    }

    #[tokio::test]
    async fn canonical_completion_bypasses_a_full_room_mailbox_via_projection_watch() {
        let key = room_key();
        let (actor_tx, _actor_rx) = mpsc::channel(1);
        actor_tx
            .try_send(TimelineActorMessage::OwnReadReceiptChanged)
            .expect("fill ordinary Room actor mailbox");
        let (projection, projection_rx) = ThreadSummaryProjectionIngress::channel();
        let mut manager = live_tail_test_manager(HashMap::from([(
            key.clone(),
            TimelineActorHandle {
                tx: actor_tx,
                control_tx: None,
                thread_summary_projection: projection,
                position_rx: None,
                task: None,
                auxiliary_tasks: Vec::new(),
                subscription_generation: None,
                enqueue_context: None,
            },
        )]));
        let actor_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply-b:test".to_owned(),
            activity_timestamp_ms: Some(200),
            activity_sender: Some("@b:test".to_owned()),
            activity_sender_label: Some("B".to_owned()),
            activity_body_preview: Some("B".to_owned()),
        };
        let refresh = {
            let mut service = manager
                .thread_root_projection_service
                .lock()
                .expect("service lock");
            assert!(matches!(
                service.observe(activity.clone()),
                ThreadRootProjectionDecision::StartFetch(_)
            ));
            service.set_canonical_root_event_ids(
                key.room_id(),
                &HashSet::from([activity.root_event_id.clone()]),
            );
            service
                .schedule_aggregate_refresh_with_canonical_root(
                    &activity,
                    AggregateRefreshCause::SelectedActivity,
                    true,
                    true,
                    false,
                )
                .expect("canonical refresh")
        };
        manager.thread_root_projection_fetches.insert(
            activity.room_id.clone(),
            activity.root_event_id.clone(),
            actor_generation,
            Some(refresh.summary_revision),
            executor::spawn(async { std::future::pending::<()>().await }),
        );

        executor::timeout(
            Duration::from_millis(100),
            manager.handle_aggregate_refresh_finished(
                key,
                actor_generation,
                refresh,
                Ok(
                    crate::threads_list::ThreadRootProjectionRefreshResult::Aggregate(
                        AuthoritativeThreadAggregate {
                            reply_count: 2,
                            latest_event_id: Some(activity.activity_event_id.clone()),
                            latest_sender: activity.activity_sender.clone(),
                            latest_sender_label: activity.activity_sender_label.clone(),
                            latest_body_preview: activity.activity_body_preview.clone(),
                            latest_timestamp_ms: activity.activity_timestamp_ms,
                        },
                    ),
                ),
            ),
        )
        .await
        .expect("manager must not wait for ordinary Room actor capacity");
        let pending = projection_rx.borrow();
        let wake = pending
            .get(&activity.root_event_id)
            .expect("accepted canonical completion wake");
        assert_eq!(wake.activity_revision, 1);
        assert_eq!(wake.summary_revision, 1);
    }

    #[tokio::test]
    async fn actor_owner_generation_remains_monotonic_across_manager_gate_recreation() {
        let key = focused_key();
        let first_gate = TimelineActorGenerationGate::default();
        let first = first_gate.activate_after_quiescence(&key).await.generation;
        drop(first_gate);

        let replacement_gate = TimelineActorGenerationGate::default();
        let replacement = replacement_gate
            .activate_after_quiescence(&key)
            .await
            .generation;
        assert!(replacement > first);
    }

    #[tokio::test]
    async fn lost_projection_delivery_replays_same_identity_until_actor_accepts_ack() {
        let key = focused_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let projection_request_id = fake_rid(91);
        let generation = TimelineGeneration(2);
        let mut acknowledged = false;

        // The first delivery is intentionally treated as lost: no ACK reaches
        // the actor. EnsureSubscribed must therefore reproject the same lease.
        assert_eq!(
            replay_projection_request_id(projection_request_id, acknowledged),
            Some(projection_request_id)
        );
        assert!(!accept_projection_ack_for_active_actor(
            &generations,
            &key,
            actor_generation,
            projection_request_id,
            generation,
            fake_rid(90),
            generation,
            &mut acknowledged,
        ));
        assert_eq!(
            replay_projection_request_id(projection_request_id, acknowledged),
            Some(projection_request_id)
        );

        assert!(accept_projection_ack_for_active_actor(
            &generations,
            &key,
            actor_generation,
            projection_request_id,
            generation,
            projection_request_id,
            generation,
            &mut acknowledged,
        ));
        assert_eq!(
            replay_projection_request_id(projection_request_id, acknowledged),
            None
        );
    }

    #[tokio::test]
    async fn initial_items_keep_projection_ack_identity_separate_from_subscribe_cause() {
        let key = focused_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let projection_request_id = fake_rid(92);
        let replay_request_id = fake_rid(93);
        let acknowledged_replay_request_id = fake_rid(94);
        let generation = TimelineGeneration(2);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let (registry, projection_service) = replay_projection_services();

        assert!(
            emit_initial_items_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &projection_service,
                &generations,
                &key,
                actor_generation,
                InitialItemsRequestIdentity::fresh(projection_request_id),
                generation,
                Vec::new(),
                Vec::new(),
            )
        );
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(found_projection_request_id),
                cause_request_id: Some(found_cause_request_id),
                ..
            })) if found_projection_request_id == projection_request_id
                && found_cause_request_id == projection_request_id
        ));

        assert!(
            emit_initial_items_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &projection_service,
                &generations,
                &key,
                actor_generation,
                InitialItemsRequestIdentity::replay(
                    projection_request_id,
                    false,
                    Some(replay_request_id),
                ),
                generation,
                Vec::new(),
                Vec::new(),
            )
        );
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(found_projection_request_id),
                cause_request_id: Some(found_cause_request_id),
                ..
            })) if found_projection_request_id == projection_request_id
                && found_cause_request_id == replay_request_id
        ));

        assert!(
            emit_initial_items_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &projection_service,
                &generations,
                &key,
                actor_generation,
                InitialItemsRequestIdentity::replay(
                    projection_request_id,
                    true,
                    Some(acknowledged_replay_request_id),
                ),
                generation,
                Vec::new(),
                Vec::new(),
            )
        );
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: None,
                cause_request_id: Some(found_cause_request_id),
                ..
            })) if found_cause_request_id == acknowledged_replay_request_id
        ));

        assert!(
            emit_initial_items_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &projection_service,
                &generations,
                &key,
                actor_generation,
                InitialItemsRequestIdentity::replay(projection_request_id, false, None),
                generation,
                Vec::new(),
                Vec::new(),
            )
        );
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(found_projection_request_id),
                cause_request_id: None,
                ..
            })) if found_projection_request_id == projection_request_id
        ));
    }

    #[tokio::test]
    async fn lost_delivery_reprojection_emits_same_core_event_under_active_actor_lease() {
        let key = focused_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let projection_request_id = fake_rid(96);
        let projection_generation = TimelineGeneration(6);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let projection = TimelineEvent::InitialItems {
            request_id: Some(projection_request_id),
            cause_request_id: Some(projection_request_id),
            key: key.clone(),
            actor_generation,
            generation: projection_generation,
            items: vec![timeline_item(
                "$focused-target:test",
                Some("synthetic"),
                "@sender:test",
                false,
            )],
        };

        assert!(emit_timeline_events_for_generation(
            &event_tx,
            &generations,
            &key,
            actor_generation,
            vec![projection.clone()],
        ));
        let _lost_first_delivery = event_rx.recv().await.expect("first projection broadcasts");

        assert!(emit_timeline_events_for_generation(
            &event_tx,
            &generations,
            &key,
            actor_generation,
            vec![projection.clone()],
        ));
        let replay = event_rx
            .recv()
            .await
            .expect("actor reprojection broadcasts");
        assert!(matches!(
            replay,
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(found_request_id),
                cause_request_id: Some(found_cause_request_id),
                key: found_key,
                actor_generation: found_actor_generation,
                generation: found_generation,
                items,
            }) if found_request_id == projection_request_id
                && found_cause_request_id == projection_request_id
                && found_key == key
                && found_actor_generation == actor_generation
                && found_generation == projection_generation
                && items.len() == 1
        ));

        let mut acknowledged = false;
        assert!(accept_projection_ack_for_active_actor(
            &generations,
            &key,
            actor_generation,
            projection_request_id,
            projection_generation,
            projection_request_id,
            projection_generation,
            &mut acknowledged,
        ));
        assert!(acknowledged);
    }

    #[tokio::test]
    async fn stale_actor_generation_cannot_emit_any_timeline_event_after_replacement() {
        let key = room_key();
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let old_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let old_lease = actor_generations
            .try_acquire(&key, old_generation)
            .expect("old actor lease");
        let replacement_gate = actor_generations.clone();
        let replacement_key = key.clone();
        let replacement = tokio::spawn(async move {
            replacement_gate
                .activate_after_quiescence(&replacement_key)
                .await
        });
        for _ in 0..10 {
            if actor_generations
                .try_acquire(&key, old_generation)
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            actor_generations
                .try_acquire(&key, old_generation)
                .is_none()
        );
        drop(old_lease);
        let new_generation = replacement.await.expect("replacement task").generation;

        let (event_tx, mut event_rx) = broadcast::channel(8);
        assert!(!emit_timeline_events_for_generation(
            &event_tx,
            &actor_generations,
            &key,
            old_generation,
            vec![TimelineEvent::ItemsUpdated {
                key: key.clone(),
                generation: TimelineGeneration(0),
                batch_id: TimelineBatchId(1),
                diffs: vec![TimelineDiff::PushBack {
                    item: timeline_item("$old-diff:test", Some("old"), "@a:test", false),
                }],
            }],
        ));
        assert!(!emit_timeline_events_for_generation(
            &event_tx,
            &actor_generations,
            &key,
            old_generation,
            vec![TimelineEvent::InitialItems {
                request_id: None,
                cause_request_id: None,
                key: key.clone(),
                actor_generation: old_generation,
                generation: TimelineGeneration(0),
                items: vec![timeline_item(
                    "$old-initial:test",
                    Some("old"),
                    "@a:test",
                    false
                )],
            }],
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        assert!(emit_timeline_events_for_generation(
            &event_tx,
            &actor_generations,
            &key,
            new_generation,
            vec![TimelineEvent::InitialItems {
                request_id: None,
                cause_request_id: None,
                key: key.clone(),
                actor_generation: new_generation,
                generation: TimelineGeneration(0),
                items: vec![timeline_item(
                    "$new-initial:test",
                    Some("new"),
                    "@a:test",
                    false
                )],
            }],
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }))
                if items.iter().any(|item| timeline_item_event_id(item) == Some("$new-initial:test"))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn timeline_event_group_uses_one_generation_lease_and_finishes_before_replacement() {
        let key = room_key();
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let old_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let lease = actor_generations
            .try_acquire(&key, old_generation)
            .expect("current actor lease");
        let replacement_gate = actor_generations.clone();
        let replacement_key = key.clone();
        let replacement = tokio::spawn(async move {
            replacement_gate
                .activate_after_quiescence(&replacement_key)
                .await
        });
        for _ in 0..10 {
            if actor_generations
                .try_acquire(&key, old_generation)
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        let (event_tx, mut event_rx) = broadcast::channel(8);
        emit_timeline_events_with_lease(
            &event_tx,
            &lease,
            vec![
                TimelineEvent::PaginationStateChanged {
                    request_id: None,
                    key: key.clone(),
                    direction: PaginationDirection::Backward,
                    state: PaginationState::Paginating,
                    prepend_expected: None,
                },
                TimelineEvent::PaginationStateChanged {
                    request_id: None,
                    key: key.clone(),
                    direction: PaginationDirection::Backward,
                    state: PaginationState::Idle,
                    prepend_expected: Some(false),
                },
            ],
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                state: PaginationState::Paginating,
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                state: PaginationState::Idle,
                ..
            }))
        ));
        assert!(
            !replacement.is_finished(),
            "the one outer lease keeps the full synchronous event group before replacement"
        );
        drop(lease);
        assert!(
            replacement.await.is_ok(),
            "replacement proceeds after the group"
        );
    }

    #[tokio::test]
    async fn fenced_diff_group_emits_neither_replay_transition_nor_items_update() {
        let key = room_key();
        let root_with_summary = || {
            let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
            root.thread_summary = Some(ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some("$summary-activity:test".to_owned()),
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(300),
            });
            root
        };
        let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
        before.timestamp_ms = Some(200);
        let mut after = timeline_item("$after:test", Some("after"), "@a:test", false);
        after.timestamp_ms = Some(400);
        let display_items = vec![before, after];
        let diffs = vec![TimelineDiff::PushBack {
            item: timeline_item("$new:test", Some("new"), "@a:test", false),
        }];

        // The actor has already processed the diff into its private mirrors.
        // SyncStarted fences its generation immediately before the single UI
        // group is committed. No old registry mutation, Clear/Ready, or diff
        // may leak through that fence.
        let fenced_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let thread_root_projection_service =
            Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let _ = refresh_replay_known_root_projections(
            &fenced_registry,
            &key,
            &[root_with_summary()],
            &display_items,
        );
        let fenced_generations = Arc::new(TimelineActorGenerationGate::default());
        let old_generation = fenced_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let _replacement_generation = fenced_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let (fenced_tx, mut fenced_rx) = broadcast::channel(8);
        assert!(
            !emit_items_updated_and_reconcile_replay_known_for_generation(
                &fenced_tx,
                &fenced_registry,
                &thread_root_projection_service,
                &fenced_generations,
                &key,
                old_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                diffs.clone(),
                &[],
                &display_items,
            ),
            "the old actor must be rejected before either half of the UI group"
        );
        assert!(matches!(
            fenced_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(
            fenced_registry
                .lock()
                .expect("registry lock")
                .get(&key)
                .is_some(),
            "a fenced actor must not mutate replay ownership either"
        );

        // A current generation commits both the canonical diff and the
        // matching source-scoped replay transition under one lease.
        let current_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let _ = refresh_replay_known_root_projections(
            &current_registry,
            &key,
            &[root_with_summary()],
            &display_items,
        );
        let current_generations = Arc::new(TimelineActorGenerationGate::default());
        let current_generation = current_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let (current_tx, mut current_rx) = broadcast::channel(8);
        assert!(
            emit_items_updated_and_reconcile_replay_known_for_generation(
                &current_tx,
                &current_registry,
                &thread_root_projection_service,
                &current_generations,
                &key,
                current_generation,
                TimelineGeneration(0),
                TimelineBatchId(0),
                diffs,
                &[],
                &display_items,
            )
        );
        assert!(matches!(
            current_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { .. }))
        ));
        assert!(matches!(
            current_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            current_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn hydration_completion_does_not_overwrite_a_current_replay_known_owner() {
        let key = room_key();
        let (action_tx, mut action_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (msg_tx, msg_rx) = mpsc::channel(8);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let replay_known_thread_root_projections = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let thread_root_projection_service =
            Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let mut manager = TimelineManagerActor {
            session: None,
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(feature = "test-hooks")]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::from([(key.clone(), test_timeline_actor_handle())]),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion: SharedSendCompletionCoordinator::default(),
            global_send_completion_observer_future: None,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::unavailable(),
            action_tx,
            event_tx,
            msg_tx,
            msg_rx,
            control_rx: None,
            navigation_projection_rx: None,
            last_navigation_projection_generation: 0,
            terminal_ingress,
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: Default::default(),
            data_dir: None,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work: AccountWorkScheduler::default(),
            thread_root_projection_service: thread_root_projection_service.clone(),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            replay_known_thread_root_projections: replay_known_thread_root_projections.clone(),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            #[cfg(test)]
            test_session_available: false,
        };
        let actor_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;

        for (root_event_id, result) in [
            (
                "$ready-root:test",
                Ok(timeline_item(
                    "$ready-root:test",
                    Some("ready"),
                    "@a:test",
                    false,
                )),
            ),
            ("$failed-root:test", Err(OperationFailureKind::Network)),
        ] {
            let activity = ThreadRootProjectionActivity {
                room_id: key.room_id().to_owned(),
                root_event_id: root_event_id.to_owned(),
                activity_event_id: format!("$activity-{root_event_id}"),
                activity_timestamp_ms: Some(300),
                activity_sender: None,
                activity_sender_label: None,
                activity_body_preview: None,
            };
            assert!(matches!(
                thread_root_projection_service
                    .lock()
                    .expect("service lock")
                    .observe(activity.clone()),
                ThreadRootProjectionDecision::StartFetch(_)
            ));
            let replay_snapshot = ThreadRootProjectionDto {
                root_event_id: root_event_id.to_owned(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready {
                    item: timeline_item(root_event_id, Some("replay root"), "@a:test", false),
                },
            };
            replay_known_thread_root_projections
                .lock()
                .expect("registry lock")
                .replace(&key, vec![replay_snapshot]);
            manager.thread_root_projection_fetches.insert(
                activity.room_id.clone(),
                activity.root_event_id.clone(),
                actor_generation,
                None,
                executor::spawn(async {}),
            );

            manager
                .handle_thread_root_projection_fetch_finished(
                    key.clone(),
                    actor_generation,
                    activity.clone(),
                    result,
                )
                .await;

            let action = action_rx.recv().await;
            if root_event_id == "$ready-root:test" {
                assert!(matches!(
                    action,
                    Some(actions) if matches!(
                        actions.as_slice(),
                        [AppAction::ThreadRootProjectionReady { root_event_id: action_root, .. }]
                        if action_root == "$ready-root:test"
                    )
                ));
            } else {
                assert!(matches!(
                    action,
                    Some(actions) if matches!(
                        actions.as_slice(),
                        [AppAction::ThreadRootProjectionFailed { root_event_id: action_root, .. }]
                        if action_root == "$failed-root:test"
                    )
                ));
            }
            assert!(matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ));
        }
        assert!(matches!(
            thread_root_projection_service
                .lock()
                .expect("service lock")
                .observe(ThreadRootProjectionActivity {
                    room_id: key.room_id().to_owned(),
                    root_event_id: "$ready-root:test".to_owned(),
                    activity_event_id: "$activity-$ready-root:test".to_owned(),
                    activity_timestamp_ms: Some(300),
                    activity_sender: None,
                    activity_sender_label: None,
                    activity_body_preview: None,
                }),
            ThreadRootProjectionDecision::Existing(record) if record.item().is_some()
        ));
        assert!(matches!(
            thread_root_projection_service
                .lock()
                .expect("service lock")
                .observe(ThreadRootProjectionActivity {
                    room_id: key.room_id().to_owned(),
                    root_event_id: "$failed-root:test".to_owned(),
                    activity_event_id: "$activity-$failed-root:test".to_owned(),
                    activity_timestamp_ms: Some(300),
                    activity_sender: None,
                    activity_sender_label: None,
                    activity_body_preview: None,
                }),
            ThreadRootProjectionDecision::Existing(record)
                if record.failure_kind() == Some(OperationFailureKind::Network)
        ));

        // With no replay-known owner, the exact same manager completion keeps
        // the existing hydration wire behavior.
        replay_known_thread_root_projections
            .lock()
            .expect("registry lock")
            .clear(&key);
        let ordinary_activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$ordinary-root:test".to_owned(),
            activity_event_id: "$ordinary-activity:test".to_owned(),
            activity_timestamp_ms: Some(301),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let _ = thread_root_projection_service
            .lock()
            .expect("service lock")
            .observe(ordinary_activity.clone());
        manager.thread_root_projection_fetches.insert(
            ordinary_activity.room_id.clone(),
            ordinary_activity.root_event_id.clone(),
            actor_generation,
            None,
            executor::spawn(async {}),
        );
        manager
            .handle_thread_root_projection_fetch_finished(
                key.clone(),
                actor_generation,
                ordinary_activity,
                Ok(timeline_item(
                    "$ordinary-root:test",
                    Some("ordinary"),
                    "@a:test",
                    false,
                )),
            )
            .await;
        assert!(matches!(
            action_rx.recv().await,
            Some(actions) if matches!(
                actions.as_slice(),
                [AppAction::ThreadRootProjectionReady { root_event_id, .. }]
                    if root_event_id == "$ordinary-root:test"
            )
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    root_event_id,
                    source: ThreadRootProjectionSourceDto::Hydration,
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            })) if root_event_id == "$ordinary-root:test"
        ));
    }

    #[tokio::test]
    async fn manager_rejects_stale_generation_hydration_start_and_completion() {
        let key = room_key();
        let mut manager =
            live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
        let mut event_rx = manager.event_tx.subscribe();
        let old_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let replacement_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        assert_ne!(old_generation, replacement_generation);
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$stale-root:test".to_owned(),
            activity_event_id: "$stale-reply:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            manager
                .thread_root_projection_service
                .lock()
                .expect("service lock")
                .observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));

        manager
            .handle_thread_root_projection_fetch_start(
                key.clone(),
                old_generation,
                None,
                vec![activity.clone()],
            )
            .await;
        assert!(!manager.thread_root_projection_fetches.contains_hydration(
            &activity.room_id,
            &activity.root_event_id,
            old_generation,
        ));

        manager.thread_root_projection_fetches.insert(
            activity.room_id.clone(),
            activity.root_event_id.clone(),
            old_generation,
            None,
            executor::spawn(async {}),
        );
        manager
            .handle_thread_root_projection_fetch_finished(
                key.clone(),
                old_generation,
                activity.clone(),
                Ok(timeline_item(
                    "$stale-root:test",
                    Some("stale"),
                    "@a:test",
                    false,
                )),
            )
            .await;
        assert!(
            manager
                .thread_root_projection_service
                .lock()
                .expect("service lock")
                .has_pending_attempt(&activity)
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn initial_items_replay_owner_group_suppresses_a_terminal_and_handoffs_exactly_once() {
        let key = room_key();
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$latest:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let _ = service
            .lock()
            .expect("service lock")
            .observe(activity.clone());
        service
            .lock()
            .expect("service lock")
            .mark_ready(
                &activity,
                timeline_item("$root:test", Some("hydrated"), "@a:test", false),
            )
            .expect("pending hydration must complete");

        let mut replay_root = timeline_item("$root:test", Some("replay"), "@a:test", false);
        replay_root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: activity.activity_timestamp_ms,
        });
        let replay_snapshot = ThreadRootProjectionDto {
            root_event_id: activity.root_event_id.clone(),
            activity_event_id: activity.activity_event_id.clone(),
            activity_timestamp_ms: activity.activity_timestamp_ms,
            retain_without_reply: true,
            source: ThreadRootProjectionSourceDto::Hydration,
            state: ThreadRootProjectionStateDto::Ready {
                item: replay_root.clone(),
            },
        };

        assert!(
            emit_initial_items_and_reconcile_replay_known_for_generation(
                &event_tx,
                &registry,
                &service,
                &actor_generations,
                &key,
                actor_generation,
                InitialItemsRequestIdentity::fresh(fake_rid(14)),
                TimelineGeneration(0),
                vec![replay_root.clone()],
                vec![replay_snapshot],
            )
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems { .. }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            }))
        ));

        let terminal = thread_root_projection_dto_from_record(
            &service
                .lock()
                .expect("service lock")
                .terminal_record(key.room_id(), &activity.root_event_id)
                .expect("terminal hydration record"),
        );
        assert!(
            !emit_hydration_terminal_unless_replay_owned(&event_tx, &registry, &key, terminal),
            "a completion attempting immediately after InitialItems must find the replay owner"
        );
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let mut exact_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
        exact_reply.thread_root = Some(activity.root_event_id.clone());
        exact_reply.timestamp_ms = activity.activity_timestamp_ms;
        let events = {
            let mut guard = registry.lock().expect("registry lock");
            let update = guard.reconcile_navigation(
                &key,
                &[replay_root, exact_reply.clone()],
                &ReplayKnownDisplayContext::from_display_items(&[exact_reply]),
            );
            replay_known_timeline_events_with_hydration_handoffs(&key, &mut guard, &service, update)
        };
        for event in events {
            let _ = event_tx.send(CoreEvent::Timeline(event));
        }
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::Hydration,
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn replay_clear_hands_back_a_hydration_terminal_that_was_emitted_before_replay_ownership() {
        let key = room_key();
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$latest:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let _ = service
            .lock()
            .expect("service lock")
            .observe(activity.clone());
        service
            .lock()
            .expect("service lock")
            .mark_ready(
                &activity,
                timeline_item("$root:test", Some("hydrated"), "@a:test", false),
            )
            .expect("pending hydration must complete");
        let hydration_terminal = thread_root_projection_dto_from_record(
            &service
                .lock()
                .expect("service lock")
                .terminal_record(key.room_id(), &activity.root_event_id)
                .expect("terminal hydration record"),
        );

        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let (event_tx, mut event_rx) = broadcast::channel(8);
        assert!(emit_hydration_terminal_unless_replay_owned(
            &event_tx,
            &registry,
            &key,
            hydration_terminal,
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::Hydration,
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            }))
        ));

        let mut replay_root = timeline_item("$root:test", Some("replay"), "@a:test", false);
        replay_root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: activity.activity_timestamp_ms,
        });
        let replay = ThreadRootProjectionDto {
            root_event_id: activity.root_event_id.clone(),
            activity_event_id: activity.activity_event_id.clone(),
            activity_timestamp_ms: activity.activity_timestamp_ms,
            retain_without_reply: true,
            source: ThreadRootProjectionSourceDto::Hydration,
            state: ThreadRootProjectionStateDto::Ready {
                item: replay_root.clone(),
            },
        };
        let initial = registry
            .lock()
            .expect("registry lock")
            .replace(&key, vec![replay]);
        assert_eq!(initial.ready.len(), 1);

        let mut exact_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
        exact_reply.thread_root = Some(activity.root_event_id.clone());
        exact_reply.timestamp_ms = activity.activity_timestamp_ms;
        let handoff = {
            let mut guard = registry.lock().expect("registry lock");
            let update = guard.reconcile_navigation(
                &key,
                &[replay_root, exact_reply.clone()],
                &ReplayKnownDisplayContext::from_display_items(&[exact_reply]),
            );
            replay_known_timeline_events_with_hydration_handoffs(&key, &mut guard, &service, update)
        };

        assert!(matches!(
            handoff.as_slice(),
            [
                TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                        state: ThreadRootProjectionStateDto::Cleared,
                        ..
                    },
                    ..
                },
                TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::Hydration,
                        state: ThreadRootProjectionStateDto::Ready { .. },
                        ..
                    },
                    ..
                }
            ]
        ));
    }

    #[tokio::test]
    async fn initial_items_forces_ready_and_failed_completion_attempts_to_wait_for_replay_ownership()
     {
        for (root_event_id, result) in [
            (
                "$ready-root:test",
                Ok(timeline_item(
                    "$ready-root:test",
                    Some("hydrated"),
                    "@a:test",
                    false,
                )),
            ),
            ("$failed-root:test", Err(OperationFailureKind::Network)),
        ] {
            let key = room_key();
            let actor_generations = Arc::new(TimelineActorGenerationGate::default());
            let actor_generation = actor_generations
                .activate_after_quiescence(&key)
                .await
                .generation;
            let activity = ThreadRootProjectionActivity {
                room_id: key.room_id().to_owned(),
                root_event_id: root_event_id.to_owned(),
                activity_event_id: format!("$latest-{root_event_id}"),
                activity_timestamp_ms: Some(300),
                activity_sender: None,
                activity_sender_label: None,
                activity_body_preview: None,
            };
            let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
            let _ = service
                .lock()
                .expect("service lock")
                .observe(activity.clone());
            match result {
                Ok(item) => service
                    .lock()
                    .expect("service lock")
                    .mark_ready(&activity, item)
                    .expect("pending hydration must complete"),
                Err(failure_kind) => service
                    .lock()
                    .expect("service lock")
                    .mark_failed(&activity, failure_kind)
                    .expect("pending hydration must complete"),
            };
            let terminal = thread_root_projection_dto_from_record(
                &service
                    .lock()
                    .expect("service lock")
                    .terminal_record(key.room_id(), &activity.root_event_id)
                    .expect("terminal hydration record"),
            );
            let mut replay_root = timeline_item(root_event_id, Some("replay"), "@a:test", false);
            replay_root.thread_summary = Some(ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some(activity.activity_event_id.clone()),
                latest_sender: None,
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: activity.activity_timestamp_ms,
            });
            let replay_snapshot = ThreadRootProjectionDto {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready {
                    item: replay_root.clone(),
                },
            };
            let registry = Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            ));
            let (event_tx, mut event_rx) = broadcast::channel(16);
            let (initial_sent_tx, initial_sent_rx) = std::sync::mpsc::channel();
            let (completion_started_tx, completion_started_rx) = std::sync::mpsc::channel();
            let completion_registry = registry.clone();
            let completion_key = key.clone();
            let completion_event_tx = event_tx.clone();
            let completion = std::thread::spawn(move || {
                initial_sent_rx
                    .recv()
                    .expect("InitialItems must release the forced completion attempt");
                completion_started_tx
                    .send(())
                    .expect("completion attempt signal");
                emit_hydration_terminal_unless_replay_owned(
                    &completion_event_tx,
                    &completion_registry,
                    &completion_key,
                    terminal,
                )
            });

            assert!(
                emit_initial_items_and_reconcile_replay_known_for_generation_with_test_hook(
                    &event_tx,
                    &registry,
                    &service,
                    &actor_generations,
                    &key,
                    actor_generation,
                    InitialItemsRequestIdentity::fresh(fake_rid(15)),
                    TimelineGeneration(0),
                    vec![replay_root.clone()],
                    vec![replay_snapshot],
                    move || {
                        initial_sent_tx
                            .send(())
                            .expect("completion must be released after InitialItems");
                        completion_started_rx.recv().expect(
                            "completion must attempt the registry while the owner group holds it",
                        );
                    },
                )
            );
            assert!(matches!(
                event_rx.try_recv(),
                Ok(CoreEvent::Timeline(TimelineEvent::InitialItems { .. }))
            ));
            assert!(matches!(
                event_rx.try_recv(),
                Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                        state: ThreadRootProjectionStateDto::Ready { .. },
                        ..
                    },
                    ..
                }))
            ));
            assert!(
                !completion
                    .join()
                    .expect("completion must finish after owner group"),
                "a terminal attempt between InitialItems and replay Ready must be suppressed"
            );
            assert!(matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ));

            let mut exact_reply =
                timeline_item(&activity.activity_event_id, Some("reply"), "@b:test", false);
            exact_reply.thread_root = Some(activity.root_event_id.clone());
            exact_reply.timestamp_ms = activity.activity_timestamp_ms;
            let events = {
                let mut guard = registry.lock().expect("registry lock");
                let update = guard.reconcile_navigation(
                    &key,
                    &[replay_root, exact_reply.clone()],
                    &ReplayKnownDisplayContext::from_display_items(&[exact_reply]),
                );
                replay_known_timeline_events_with_hydration_handoffs(
                    &key, &mut guard, &service, update,
                )
            };
            for event in events {
                let _ = event_tx.send(CoreEvent::Timeline(event));
            }
            assert!(matches!(
                event_rx.try_recv(),
                Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                        state: ThreadRootProjectionStateDto::Cleared,
                        ..
                    },
                    ..
                }))
            ));
            let handoff = event_rx
                .try_recv()
                .expect("one suppressed terminal handoff");
            assert!(matches!(
                handoff,
                CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::Hydration,
                        state: ThreadRootProjectionStateDto::Ready { .. }
                            | ThreadRootProjectionStateDto::Failed { .. },
                        ..
                    },
                    ..
                })
            ));
            assert!(matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ));
        }
    }

    #[test]
    fn hydration_terminal_cannot_overtake_a_replay_known_ready_in_the_event_stream() {
        let key = room_key();
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));

        for hydration_state in [
            ThreadRootProjectionStateDto::Ready {
                item: timeline_item("$root:test", Some("hydrated"), "@a:test", false),
            },
            ThreadRootProjectionStateDto::Failed {
                failure_kind: OperationFailureKind::Network,
            },
        ] {
            let root_event_id = "$root:test".to_owned();
            let replay_snapshot = ThreadRootProjectionDto {
                root_event_id: root_event_id.clone(),
                activity_event_id: "$latest:test".to_owned(),
                activity_timestamp_ms: Some(300),
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready {
                    item: timeline_item("$root:test", Some("replay"), "@a:test", false),
                },
            };
            let hydration_terminal = ThreadRootProjectionDto {
                root_event_id: root_event_id.clone(),
                activity_event_id: "$latest:test".to_owned(),
                activity_timestamp_ms: Some(300),
                retain_without_reply: false,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: hydration_state,
            };

            // This guard models the actor's short replay ownership section.
            // A concurrent manager completion may start, but must not decide
            // ownership or emit until the actor has published its Ready.
            let mut actor_registry = registry.lock().expect("registry lock");
            let registry_for_completion = registry.clone();
            let event_tx_for_completion = event_tx.clone();
            let key_for_completion = key.clone();
            let (completion_started_tx, completion_started_rx) = std::sync::mpsc::channel();
            let completion = std::thread::spawn(move || {
                completion_started_tx
                    .send(())
                    .expect("completion test coordination");
                emit_hydration_terminal_unless_replay_owned(
                    &event_tx_for_completion,
                    &registry_for_completion,
                    &key_for_completion,
                    hydration_terminal,
                )
            });
            completion_started_rx
                .recv()
                .expect("completion must attempt the shared ownership boundary");

            let update = actor_registry.replace(&key, vec![replay_snapshot]);
            for event in replay_known_timeline_events(&key, update) {
                let _ = event_tx.send(CoreEvent::Timeline(event));
            }
            drop(actor_registry);

            assert!(
                !completion.join().expect("completion worker must finish"),
                "a replay-known owner must suppress both Ready and Failed hydration terminals"
            );
            assert!(matches!(
                event_rx.try_recv(),
                Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                        state: ThreadRootProjectionStateDto::Ready { .. },
                        ..
                    },
                    ..
                }))
            ));
            assert!(matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ));

            registry.lock().expect("registry lock").clear(&key);
        }
    }

    #[test]
    fn replay_owner_clear_handoffs_the_retained_hydration_terminal_to_the_exact_reply_slot() {
        let key = room_key();
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$latest:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        assert!(matches!(
            service
                .lock()
                .expect("service lock")
                .observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        service
            .lock()
            .expect("service lock")
            .mark_ready(
                &activity,
                timeline_item("$root:test", Some("hydrated"), "@a:test", false),
            )
            .expect("pending hydration must complete");

        let mut root = timeline_item("$root:test", Some("root"), "@a:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: activity.activity_timestamp_ms,
        });
        let mut exact_latest_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
        exact_latest_reply.thread_root = Some(activity.root_event_id.clone());
        exact_latest_reply.timestamp_ms = activity.activity_timestamp_ms;

        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let replay_snapshot = ThreadRootProjectionDto {
            root_event_id: activity.root_event_id.clone(),
            activity_event_id: activity.activity_event_id.clone(),
            activity_timestamp_ms: activity.activity_timestamp_ms,
            retain_without_reply: true,
            source: ThreadRootProjectionSourceDto::Hydration,
            state: ThreadRootProjectionStateDto::Ready { item: root.clone() },
        };
        registry
            .lock()
            .expect("registry lock")
            .replace(&key, vec![replay_snapshot]);

        let (event_tx, mut event_rx) = broadcast::channel(8);
        let suppressed_terminal = thread_root_projection_dto_from_record(
            &service
                .lock()
                .expect("service lock")
                .terminal_record(key.room_id(), &activity.root_event_id)
                .expect("completed hydration must remain retained while replay owns it"),
        );
        assert!(
            !emit_hydration_terminal_unless_replay_owned(
                &event_tx,
                &registry,
                &key,
                suppressed_terminal,
            ),
            "the terminal must be recorded for a later one-time handoff, not emitted now"
        );

        let mut registry_guard = registry.lock().expect("registry lock");
        let update = registry_guard.reconcile_navigation(
            &key,
            &[root, exact_latest_reply.clone()],
            &ReplayKnownDisplayContext::from_display_items(&[exact_latest_reply]),
        );
        assert!(matches!(
            update.stale.as_slice(),
            [ReplayKnownThreadRootProjection { root_event_id, .. }]
                if root_event_id == "$root:test"
        ));

        let events = replay_known_timeline_events_with_hydration_handoffs(
            &key,
            &mut registry_guard,
            &service,
            update,
        );
        for event in events {
            let _ = event_tx.send(CoreEvent::Timeline(event));
        }
        drop(registry_guard);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::Hydration,
                    state: ThreadRootProjectionStateDto::Ready { item },
                    ..
                },
                ..
            })) if item.body.as_deref() == Some("hydrated")
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            service.lock().expect("service lock").observe(activity),
            ThreadRootProjectionDecision::Existing(record) if record.item().is_some()
        ));
    }

    #[tokio::test]
    async fn replay_owner_removal_handoffs_an_existing_terminal_reemit_once() {
        let key = room_key();
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$removed-root:test".to_owned(),
            activity_event_id: "$latest:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let _ = service
            .lock()
            .expect("service lock")
            .observe(activity.clone());
        service
            .lock()
            .expect("service lock")
            .mark_ready(
                &activity,
                timeline_item("$removed-root:test", Some("hydrated"), "@a:test", false),
            )
            .expect("pending hydration must complete");
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        registry.lock().expect("registry lock").replace(
            &key,
            vec![ThreadRootProjectionDto {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready {
                    item: timeline_item("$removed-root:test", Some("replay"), "@a:test", false),
                },
            }],
        );
        let terminal = thread_root_projection_dto_from_record(
            &service
                .lock()
                .expect("service lock")
                .terminal_record(key.room_id(), &activity.root_event_id)
                .expect("terminal record"),
        );
        let (event_tx, mut event_rx) = broadcast::channel(8);
        assert!(!emit_hydration_terminal_unless_replay_owned(
            &event_tx,
            &registry,
            &key,
            terminal.clone(),
        ));
        let _lease = actor_generations
            .try_acquire(&key, actor_generation)
            .expect("current actor lease");
        assert!(
            !emit_hydration_terminal_unless_replay_owned(&event_tx, &registry, &key, terminal),
            "an Existing terminal reemit must share the same one-time suppression marker"
        );

        let mut exact_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
        exact_reply.thread_root = Some(activity.root_event_id.clone());
        exact_reply.timestamp_ms = activity.activity_timestamp_ms;
        let events = {
            let mut guard = registry.lock().expect("registry lock");
            let update = guard.reconcile_navigation(
                &key,
                &[exact_reply.clone()],
                &ReplayKnownDisplayContext::from_display_items(&[exact_reply]),
            );
            replay_known_timeline_events_with_hydration_handoffs(&key, &mut guard, &service, update)
        };
        for event in events {
            let _ = event_tx.send(CoreEvent::Timeline(event));
        }
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::Hydration,
                    state: ThreadRootProjectionStateDto::Ready { .. },
                    ..
                },
                ..
            }))
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn replay_owner_clear_does_not_reemit_an_unsuppressed_hydration_terminal() {
        let key = room_key();
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$latest:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let _ = service
            .lock()
            .expect("service lock")
            .observe(activity.clone());
        service
            .lock()
            .expect("service lock")
            .mark_ready(
                &activity,
                timeline_item("$root:test", Some("hydrated"), "@a:test", false),
            )
            .expect("pending hydration must complete");
        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        registry.lock().expect("registry lock").replace(
            &key,
            vec![ThreadRootProjectionDto {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready {
                    item: timeline_item("$root:test", Some("replay"), "@a:test", false),
                },
            }],
        );

        let mut registry_guard = registry.lock().expect("registry lock");
        let update = registry_guard.replace(&key, Vec::new());
        let events = replay_known_timeline_events_with_hydration_handoffs(
            &key,
            &mut registry_guard,
            &service,
            update,
        );
        assert!(matches!(
            events.as_slice(),
            [TimelineEvent::ThreadRootProjection {
                projection: ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
                ..
            }]
        ));
    }

    #[test]
    fn replay_owner_clear_handoffs_the_retained_hydration_failure_without_refetching() {
        let key = room_key();
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$failed-root:test".to_owned(),
            activity_event_id: "$latest:test".to_owned(),
            activity_timestamp_ms: Some(300),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        assert!(matches!(
            service
                .lock()
                .expect("service lock")
                .observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        service
            .lock()
            .expect("service lock")
            .mark_failed(&activity, OperationFailureKind::Network)
            .expect("pending hydration must fail terminally");

        let mut root = timeline_item("$failed-root:test", Some("root"), "@a:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: activity.activity_timestamp_ms,
        });
        let mut exact_latest_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
        exact_latest_reply.thread_root = Some(activity.root_event_id.clone());
        exact_latest_reply.timestamp_ms = activity.activity_timestamp_ms;

        let registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        registry.lock().expect("registry lock").replace(
            &key,
            vec![ThreadRootProjectionDto {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
                retain_without_reply: true,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Ready { item: root.clone() },
            }],
        );
        let (event_tx, _event_rx) = broadcast::channel(8);
        assert!(!emit_hydration_terminal_unless_replay_owned(
            &event_tx,
            &registry,
            &key,
            ThreadRootProjectionDto {
                root_event_id: activity.root_event_id.clone(),
                activity_event_id: activity.activity_event_id.clone(),
                activity_timestamp_ms: activity.activity_timestamp_ms,
                retain_without_reply: false,
                source: ThreadRootProjectionSourceDto::Hydration,
                state: ThreadRootProjectionStateDto::Failed {
                    failure_kind: OperationFailureKind::Network,
                },
            },
        ));

        let mut registry_guard = registry.lock().expect("registry lock");
        let update = registry_guard.reconcile_navigation(
            &key,
            &[root, exact_latest_reply.clone()],
            &ReplayKnownDisplayContext::from_display_items(&[exact_latest_reply]),
        );
        let events = replay_known_timeline_events_with_hydration_handoffs(
            &key,
            &mut registry_guard,
            &service,
            update,
        );
        drop(registry_guard);

        assert!(matches!(
            events.as_slice(),
            [
                TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::ReplayKnown { .. },
                        state: ThreadRootProjectionStateDto::Cleared,
                        ..
                    },
                    ..
                },
                TimelineEvent::ThreadRootProjection {
                    projection: ThreadRootProjectionDto {
                        source: ThreadRootProjectionSourceDto::Hydration,
                        state: ThreadRootProjectionStateDto::Failed {
                            failure_kind: OperationFailureKind::Network,
                        },
                        ..
                    },
                    ..
                }
            ]
        ));
        assert!(matches!(
            service.lock().expect("service lock").observe(activity),
            ThreadRootProjectionDecision::Existing(record)
                if record.failure_kind() == Some(OperationFailureKind::Network)
        ));
    }

    #[test]
    fn replay_known_epoch_never_exceeds_the_javascript_safe_integer_or_reuses_an_owner_epoch() {
        let key = room_key();
        let mut registry = ReplayKnownThreadRootProjectionRegistry::default();
        registry.next_epoch = JAVASCRIPT_SAFE_INTEGER_MAX;
        let projection = |root_event_id: &str, activity_event_id: &str| ThreadRootProjectionDto {
            root_event_id: root_event_id.to_owned(),
            activity_event_id: activity_event_id.to_owned(),
            activity_timestamp_ms: Some(300),
            retain_without_reply: true,
            source: ThreadRootProjectionSourceDto::Hydration,
            state: ThreadRootProjectionStateDto::Ready {
                item: timeline_item(root_event_id, Some("root"), "@a:test", false),
            },
        };

        let initial = registry.replace(
            &key,
            vec![
                projection("$max-owner:test", "$max-activity:test"),
                projection("$wrapped-owner:test", "$wrapped-activity:test"),
            ],
        );
        assert!(matches!(
            initial.ready.as_slice(),
            [
                ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown {
                        epoch: JAVASCRIPT_SAFE_INTEGER_MAX
                    },
                    ..
                },
                ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    ..
                },
            ]
        ));

        // Simulate another complete sequence wrap while both prior owners are
        // current. The revised root must get a distinct safe epoch rather
        // than colliding with either a stale Clear or the still-current root.
        registry.next_epoch = JAVASCRIPT_SAFE_INTEGER_MAX;
        let replaced = registry.replace(
            &key,
            vec![
                projection("$max-owner:test", "$replacement-activity:test"),
                projection("$wrapped-owner:test", "$wrapped-activity:test"),
            ],
        );
        assert!(matches!(
            replaced.stale.as_slice(),
            [ReplayKnownThreadRootProjection {
                source: ThreadRootProjectionSourceDto::ReplayKnown {
                    epoch: JAVASCRIPT_SAFE_INTEGER_MAX
                },
                ..
            }]
        ));
        assert!(matches!(
            replaced.ready.as_slice(),
            [
                ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 2 },
                    ..
                },
                ThreadRootProjectionDto {
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    ..
                },
            ]
        ));
        for epoch in replaced
            .ready
            .iter()
            .filter_map(|projection| match projection.source {
                ThreadRootProjectionSourceDto::ReplayKnown { epoch } => Some(epoch),
                ThreadRootProjectionSourceDto::Hydration => None,
            })
        {
            assert!((1..=JAVASCRIPT_SAFE_INTEGER_MAX).contains(&epoch));
        }
    }

    #[test]
    fn replay_known_registry_lifecycle_helpers_cover_actor_refresh_paths() {
        let cases = [
            (
                "spawn",
                item_body(include_str!("actor.rs"), "async fn spawn("),
            ),
            (
                "replay",
                item_body(
                    include_str!("navigation.rs"),
                    "fn handle_replay_initial_items",
                ),
            ),
            (
                "send_queue_lag",
                item_body(
                    include_str!("outbound_send.rs"),
                    "async fn handle_send_queue_lagged",
                ),
            ),
            (
                "queue_overflow",
                item_body(include_str!("relay.rs"), "async fn handle_relay_overflow"),
            ),
        ];
        for (name, section) in cases {
            assert!(
                section.contains("emit_initial_items_and_reconcile_replay_known_for_generation")
                    || section.contains("commit_prepared_initial_window_for_generation")
                    || (name == "queue_overflow"
                        && (section.contains("emit_relay_recovery_snapshot")
                            || section.contains("commit_authoritative_recovery_window"))),
                "{name} must publish InitialItems and replay-known ownership in one group"
            );
        }
        let diff_handler = item_body(include_str!("relay.rs"), "async fn handle_diff_batch");
        let diff_commit = "commit_sdk_batch_for_generation";
        assert!(
            diff_handler.contains(diff_commit),
            "normal diffs must commit replay-known ownership beside ItemsUpdated"
        );
        assert!(
            diff_handler
                .find("self.maybe_hydrate_missing_thread_roots(")
                .zip(diff_handler.find(diff_commit))
                .is_some_and(|(hydration, commit)| commit < hydration),
            "the canonical ItemsUpdated group must reach the store before hydration emits Pending"
        );
        let restore_finish = item_body(include_str!("navigation.rs"), "fn finish_anchor_restore");
        assert!(
            restore_finish.contains("publish_restore_settlement"),
            "restore terminal must use the same atomic terminal group"
        );
        let actor_message_handler = item_body(include_str!("actor.rs"), "async fn handle_msg");
        assert!(
            actor_message_handler.contains("self.restore_anchor.is_none()"),
            "deferred hydration must not consume its marker while a restore buffer still owns canonical diffs"
        );
    }

    #[tokio::test]
    async fn room_unsubscribe_emits_core_clear_for_replay_known_roots_before_a_revisit() {
        let key = room_key();
        let root = timeline_item(
            "$replay-known-root:test",
            Some("root"),
            "@alice:test",
            false,
        );
        let projection = ThreadRootProjectionDto {
            root_event_id: "$replay-known-root:test".to_owned(),
            activity_event_id: "$replay-known-activity:test".to_owned(),
            activity_timestamp_ms: Some(100),
            retain_without_reply: true,
            source: ThreadRootProjectionSourceDto::Hydration,
            state: ThreadRootProjectionStateDto::Ready { item: root },
        };
        let (action_tx, _action_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (msg_tx, msg_rx) = mpsc::channel(8);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let replay_known_thread_root_projections = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        replay_known_thread_root_projections
            .lock()
            .expect("registry lock")
            .replace(&key, vec![projection]);
        let mut manager = TimelineManagerActor {
            session: None,
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(feature = "test-hooks")]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::from([(key.clone(), test_timeline_actor_handle())]),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion: SharedSendCompletionCoordinator::default(),
            global_send_completion_observer_future: None,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::unavailable(),
            action_tx,
            event_tx,
            msg_tx,
            msg_rx,
            control_rx: None,
            navigation_projection_rx: None,
            last_navigation_projection_generation: 0,
            terminal_ingress,
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: Default::default(),
            data_dir: None,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work: AccountWorkScheduler::default(),
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            replay_known_thread_root_projections: replay_known_thread_root_projections.clone(),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            #[cfg(test)]
            test_session_available: false,
        };

        manager
            .handle_command(TimelineCommand::Unsubscribe {
                request_id: fake_rid(99),
                key: key.clone(),
            })
            .await;

        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::ThreadRootProjection {
                key: cleared_key,
                projection: ThreadRootProjectionDto {
                    root_event_id,
                    activity_event_id,
                    retain_without_reply: false,
                    source: ThreadRootProjectionSourceDto::ReplayKnown { epoch: 1 },
                    state: ThreadRootProjectionStateDto::Cleared,
                    ..
                },
            }))
                if cleared_key == key
                    && root_event_id == "$replay-known-root:test"
                    && activity_event_id == "$replay-known-activity:test"
        ));
        assert!(!manager.timelines.contains_key(&key));
        assert!(
            replay_known_thread_root_projections
                .lock()
                .expect("registry lock")
                .is_empty()
        );

        // Revisit starts from an empty lifecycle registry, so an old retained
        // snapshot has no route back into the display store.
        manager
            .timelines
            .insert(key.clone(), test_timeline_actor_handle());
        assert!(
            replay_known_thread_root_projections
                .lock()
                .expect("registry lock")
                .get(&key)
                .is_none()
        );
    }

    fn timeline_message_item(event_id: &str, sender: &str) -> TimelineItem {
        TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: Some(sender.to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("body".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }
    }

    fn thread_reply_item(event_id: &str, sender: &str, root_event_id: &str) -> TimelineItem {
        TimelineItem {
            thread_root: Some(root_event_id.to_owned()),
            ..timeline_message_item(event_id, sender)
        }
    }

    #[test]
    fn room_live_timeline_focus_includes_threaded_events() {
        let build = item_body(
            include_str!("manager.rs"),
            "async fn build_timeline_actor_handle",
        );
        let focus_source = build
            .split("let focus = match &key.kind")
            .nth(1)
            .expect("subscribe focus match should exist");
        let room_focus = focus_source
            .split("TimelineKind::Room")
            .nth(1)
            .expect("room timeline focus arm should exist")
            .split("TimelineKind::Thread")
            .next()
            .expect("thread timeline focus arm should follow room arm");
        assert!(
            room_focus.contains("hide_threaded_events: false"),
            "room live timelines must keep threaded replies in canonical SDK items so the display projection can represent them"
        );
    }

    #[test]
    fn old_root_reply_reaches_bounded_room_projection_hydration_without_pagination() {
        let mut reply = timeline_item(
            "$latest-reply:test",
            Some("new reply"),
            "@alice:test",
            false,
        );
        reply.timestamp_ms = Some(1_700_000_100_000);
        reply.thread_root = Some("$old-root:test".to_owned());

        let activity = thread_root_projection_activity_from_item("!room:test", &reply)
            .expect("a canonical Room reply must be observable for root hydration");
        assert_eq!(activity.root_event_id, "$old-root:test");
        assert_eq!(activity.activity_event_id, "$latest-reply:test");
        assert_eq!(activity.activity_timestamp_ms, Some(1_700_000_100_000));

        let source = include_str!("thread_projection.rs");
        let production = source
            .split("\nmod tests")
            .next()
            .expect("production source before tests");
        let hydration = production
            .split("fn maybe_hydrate_missing_thread_roots")
            .nth(1)
            .expect("Room root hydration detection must exist")
            .split("async fn handle_ignored_users_updated")
            .next()
            .expect("root hydration handler boundary");
        assert!(
            hydration.contains("missing_activities")
                && hydration.contains("commit_prepared_thread_root_hydration_for_generation"),
            "an absent root must request one manager-owned bounded fetch"
        );
        let commit = production
            .split("async fn commit_prepared_thread_root_hydration_for_generation")
            .nth(1)
            .expect("generation-scoped hydration commit must exist")
            .split("fn thread_root_projection_action_from_record")
            .next()
            .expect("hydration commit boundary");
        assert!(
            commit.contains("reserve_owned().await")
                && commit.contains("ThreadRootProjectionDecision::StartFetch")
                && commit.contains("schedule_aggregate_refresh")
                && commit.contains("TimelineMessage::StartAggregateRefresh")
                && commit.contains("actor_generation")
                && !commit.contains("TimelineMessage::StartThreadRootProjectionFetch")
                && !commit.contains("try_send"),
            "projection state and tagged aggregate refreshes must commit reliably for one actor generation"
        );
        assert!(
            !hydration.contains("paginate_backwards(")
                && !hydration.contains("handle_restore_timeline_anchor("),
            "root hydration must not initiate Room pagination or anchor materialization"
        );
    }

    #[tokio::test]
    async fn root_projection_actions_wait_for_reducer_capacity_instead_of_dropping() {
        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .try_send(vec![AppAction::ThreadRootProjectionsCleared {
                room_id: "!already-buffered:test".to_owned(),
            }])
            .expect("fill the reducer channel");

        let reliable_tx = action_tx.clone();
        let delivery = tokio::spawn(async move {
            emit_app_action_reliable(
                &reliable_tx,
                AppAction::ThreadRootProjectionsCleared {
                    room_id: "!must-arrive:test".to_owned(),
                },
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(
            !delivery.is_finished(),
            "the reliable sender must wait behind a full channel, not discard the projection transition"
        );
        let _ = action_rx.recv().await.expect("drain buffered action");
        assert!(delivery.await.expect("delivery task"));
        assert!(matches!(
            action_rx.recv().await,
            Some(actions) if matches!(
                actions.as_slice(),
                [AppAction::ThreadRootProjectionsCleared { room_id }]
                    if room_id == "!must-arrive:test"
            )
        ));
    }

    #[tokio::test]
    async fn root_projection_fetch_registry_aborts_room_workers_and_rejects_late_completion() {
        struct CancellationProbe(Option<tokio::sync::oneshot::Sender<()>>);

        impl Drop for CancellationProbe {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }

        let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = executor::spawn(async move {
            let _probe = CancellationProbe(Some(cancelled_tx));
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let mut registry = ThreadRootProjectionFetchRegistry::default();
        registry.insert(
            "!room:test".to_owned(),
            "$root:test".to_owned(),
            7,
            None,
            task,
        );
        started_rx
            .await
            .expect("worker must be in flight before cancellation");

        assert_eq!(registry.abort_room("!room:test"), 1);
        tokio::time::timeout(Duration::from_secs(1), cancelled_rx)
            .await
            .expect("abort must end the in-flight hydration worker")
            .expect("worker cancellation probe should be delivered");
        assert!(
            !registry.take_completion("!room:test", "$root:test", 7, None),
            "a completion queued before unsubscribe must not publish a stale terminal state"
        );
    }

    #[tokio::test]
    async fn aggregate_start_preserves_fetch_finished_worker_and_failed_hydration_terminal() {
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let session = Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
            client.clone(),
            koushi_state::SessionInfo {
                homeserver: server.server().uri(),
                user_id: client.user_id().expect("synthetic user id").to_string(),
                device_id: client.device_id().expect("synthetic device id").to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let key = room_key();
        let mut manager =
            live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
        manager.session = Some(session);
        let actor_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let activity = ThreadRootProjectionActivity {
            room_id: key.room_id().to_owned(),
            root_event_id: "$failed-root:test".to_owned(),
            activity_event_id: "$reply:test".to_owned(),
            activity_timestamp_ms: Some(100),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        let refresh = {
            let mut service = manager
                .thread_root_projection_service
                .lock()
                .expect("service lock");
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
                .expect("initial aggregate refresh");
            service.mark_failed(&activity, OperationFailureKind::NotFound);
            refresh
        };

        // FetchFinished has removed hydration and started this exact aggregate
        // worker before the original StartAggregateRefresh reaches the FIFO.
        manager.thread_root_projection_fetches.insert(
            activity.room_id.clone(),
            activity.root_event_id.clone(),
            actor_generation,
            None,
            executor::spawn(async { std::future::pending::<()>().await }),
        );
        assert!(manager.thread_root_projection_fetches.take_completion(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
            None,
        ));
        manager.thread_root_projection_fetches.insert(
            activity.room_id.clone(),
            activity.root_event_id.clone(),
            actor_generation,
            Some(refresh.summary_revision),
            executor::spawn(async { std::future::pending::<()>().await }),
        );
        assert!(manager.thread_root_projection_fetches.contains_aggregate(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
            refresh.summary_revision,
        ));

        manager
            .handle_aggregate_refresh_start(
                key.clone(),
                actor_generation,
                None,
                vec![refresh.clone()],
            )
            .await;
        assert!(manager.thread_root_projection_fetches.contains_aggregate(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
            refresh.summary_revision,
        ));
        assert!(!manager.thread_root_projection_fetches.contains_hydration(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
        ));

        assert!(manager.thread_root_projection_fetches.take_completion(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
            Some(refresh.summary_revision),
        ));
        assert!(matches!(
            manager
                .thread_root_projection_service
                .lock()
                .expect("service lock")
                .complete_refresh(&refresh, Err(OperationFailureKind::Network)),
            crate::threads_list::ThreadRootProjectionCompletion::Updated(record)
                if record.failure_kind() == Some(OperationFailureKind::Network)
        ));
        let service = manager
            .thread_root_projection_service
            .lock()
            .expect("service lock");
        assert!(!service.has_pending_attempt(&activity));
        drop(service);
        manager
            .handle_aggregate_refresh_start(key, actor_generation, None, vec![refresh])
            .await;
        assert!(!manager.thread_root_projection_fetches.contains_hydration(
            &activity.room_id,
            &activity.root_event_id,
            actor_generation,
        ));
    }

    #[test]
    fn loaded_old_root_raw_event_projects_renderable_snapshot_with_latest_activity_identity() {
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:test".to_owned(),
            root_event_id: "$old-root:test".to_owned(),
            activity_event_id: "$latest-reply:test".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: Some("@latest:test".to_owned()),
            activity_sender_label: Some("Latest".to_owned()),
            activity_body_preview: Some("live reply preview".to_owned()),
        };
        let raw = serde_json::json!({
            "type": "m.room.message",
            "event_id": "$old-root:test",
            "sender": "@alice:test",
            "origin_server_ts": 1_700_000_000_000_u64,
            "content": { "msgtype": "m.text", "body": "old root body" },
            "unsigned": {
                "m.relations": {
                    "m.thread": {
                        "count": 3,
                        "latest_event": {
                            "event_id": "$stale-latest:test",
                            "sender": "@bob:test",
                            "origin_server_ts": 1_700_000_050_000_u64,
                            "content": { "body": "stale preview" }
                        }
                    }
                }
            }
        });

        let item = thread_root_projection_item_from_raw(&room_key(), None, &activity, raw)
            .expect("valid loaded root must yield a renderable snapshot");
        assert_eq!(timeline_item_event_id(&item), Some("$old-root:test"));
        assert_eq!(item.body.as_deref(), Some("old root body"));
        assert_eq!(item.timestamp_ms, Some(1_700_000_000_000));
        assert_eq!(item.thread_root, None);
        assert_eq!(
            item.thread_summary
                .as_ref()
                .and_then(|summary| summary.latest_event_id.as_deref()),
            Some("$stale-latest:test"),
            "raw bundled relation data is only provisional before Task A resolution"
        );
        assert_eq!(
            item.thread_summary
                .as_ref()
                .map(|summary| summary.reply_count),
            Some(3)
        );

        let authoritative = thread_root_item_with_authoritative_aggregate(
            &item,
            &AuthoritativeThreadAggregate {
                reply_count: 4,
                latest_event_id: Some(activity.activity_event_id.clone()),
                latest_sender: activity.activity_sender.clone(),
                latest_sender_label: activity.activity_sender_label.clone(),
                latest_body_preview: activity.activity_body_preview.clone(),
                latest_timestamp_ms: activity.activity_timestamp_ms,
            },
        );
        assert_eq!(
            authoritative
                .thread_summary
                .as_ref()
                .and_then(|summary| summary.latest_event_id.as_deref()),
            Some("$latest-reply:test")
        );
        assert_eq!(
            authoritative
                .thread_summary
                .as_ref()
                .map(|summary| summary.reply_count),
            Some(4)
        );
    }

    #[test]
    fn loaded_old_root_reuses_message_projection_for_formatted_spoiler_and_media_content() {
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:test".to_owned(),
            root_event_id: "$old-root:test".to_owned(),
            activity_event_id: "$latest-reply:test".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: Some("@latest:test".to_owned()),
            activity_sender_label: Some("Latest".to_owned()),
            activity_body_preview: Some("live reply preview".to_owned()),
        };
        let raw = serde_json::json!({
            "event_id": "$old-root:test",
            "sender": "@alice:test",
            "origin_server_ts": 1_700_000_000_000u64,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.image",
                "body": "caption ||secret||",
                "filename": "image.png",
                "format": "org.matrix.custom.html",
                "formatted_body": "<strong>caption</strong> <span data-mx-spoiler=\"reason\">secret</span>",
                "url": "mxc://test/media",
                "info": {
                    "mimetype": "image/png",
                    "size": 42,
                    "w": 640,
                    "h": 480
                }
            }
        });

        let item = thread_root_projection_item_from_raw(&room_key(), None, &activity, raw)
            .expect("loaded image root must keep normal render fields");

        assert_eq!(
            item.formatted
                .as_ref()
                .map(|formatted| formatted.plain_text.as_str()),
            Some("caption secret")
        );
        assert!(
            item.spoiler_spans
                .iter()
                .any(|span| span.reason.as_deref() == Some("reason"))
        );
        let media = item
            .media
            .expect("image root must retain media renderer data");
        assert_eq!(media.kind, TimelineMediaKind::Image);
        assert_eq!(media.source.mxc_uri, "mxc://test/media");
        assert_eq!(media.width, Some(640));
        assert_eq!(media.height, Some(480));
    }

    #[test]
    fn loaded_old_root_reuses_message_projection_for_file_audio_and_sticker_content() {
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:test".to_owned(),
            root_event_id: "$old-root:test".to_owned(),
            activity_event_id: "$latest-reply:test".to_owned(),
            activity_timestamp_ms: Some(1_700_000_100_000),
            activity_sender: Some("@latest:test".to_owned()),
            activity_sender_label: Some("Latest".to_owned()),
            activity_body_preview: Some("live reply preview".to_owned()),
        };

        let file = thread_root_projection_item_from_raw(
            &room_key(),
            None,
            &activity,
            serde_json::json!({
                "event_id": "$old-root:test",
                "sender": "@alice:test",
                "origin_server_ts": 1_700_000_000_000u64,
                "type": "m.room.message",
                "content": {
                    "msgtype": "m.file", "body": "report.pdf", "url": "mxc://test/file",
                    "filename": "report.pdf", "info": { "mimetype": "application/pdf", "size": 4 }
                }
            }),
        )
        .expect("loaded file root should use the standard file projection");
        assert_eq!(
            file.media.as_ref().map(|media| media.kind),
            Some(TimelineMediaKind::File)
        );
        assert_eq!(
            file.media.as_ref().map(|media| media.filename.as_str()),
            Some("report.pdf")
        );

        let audio = thread_root_projection_item_from_raw(
            &room_key(),
            None,
            &activity,
            serde_json::json!({
                "event_id": "$old-root:test",
                "sender": "@alice:test",
                "origin_server_ts": 1_700_000_000_000u64,
                "type": "m.room.message",
                "content": {
                    "msgtype": "m.audio", "body": "voice.ogg", "url": "mxc://test/audio",
                    "info": { "mimetype": "audio/ogg", "size": 4 }
                }
            }),
        )
        .expect("loaded audio root should use the standard audio projection");
        assert_eq!(
            audio.media.as_ref().map(|media| media.kind),
            Some(TimelineMediaKind::Audio)
        );

        let sticker = thread_root_projection_item_from_raw(
            &room_key(),
            None,
            &activity,
            serde_json::json!({
                "event_id": "$old-root:test",
                "sender": "@alice:test",
                "origin_server_ts": 1_700_000_000_000u64,
                "type": "m.sticker",
                "content": {
                    "body": "party", "url": "mxc://test/sticker",
                    "info": { "mimetype": "image/png" }
                }
            }),
        )
        .expect("loaded sticker root should use the standard sticker projection");
        assert_eq!(sticker.body.as_deref(), Some("party"));
    }

    #[test]
    fn cached_root_relations_project_reactions_without_network_or_unrelated_targets() {
        let relations = vec![
            serde_json::json!({
                "event_id": "$reaction-a:test", "sender": "@alice:test", "type": "m.reaction",
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$old-root:test", "key": "👍" } }
            }),
            serde_json::json!({
                "event_id": "$reaction-b:test", "sender": "@me:test", "type": "m.reaction",
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$old-root:test", "key": "👍" } }
            }),
            serde_json::json!({
                "event_id": "$different-target:test", "sender": "@eve:test", "type": "m.reaction",
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$other-root:test", "key": "👍" } }
            }),
        ];
        let own_user_id = matrix_sdk::ruma::UserId::parse("@me:test").expect("valid own user");

        let reactions = reaction_groups_from_cached_relation_events(
            relations,
            "$old-root:test",
            Some(own_user_id.as_ref()),
        );

        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].key, "👍");
        assert_eq!(reactions[0].count, 2);
        assert!(reactions[0].reacted_by_me);
        assert_eq!(
            reactions[0].my_reaction_event_id.as_deref(),
            Some("$reaction-b:test")
        );
    }

    #[test]
    fn sdk_projection_reads_thread_contract_accessors() {
        let source = include_str!("item_projection.rs");
        let projection_source = source
            .split("pub fn sdk_item_to_timeline_item")
            .nth(1)
            .expect("sdk projection function should exist")
            .split("pub(crate) fn timeline_item_can_react")
            .next()
            .expect("projection helper should follow sdk projection");
        let compact_projection_source: String = projection_source
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();

        assert!(
            compact_projection_source.contains("content().thread_root()"),
            "timeline item projection must read SDK thread_root"
        );
        assert!(
            compact_projection_source.contains("content().thread_summary()"),
            "timeline item projection must read SDK thread_summary"
        );
    }

    #[test]
    fn thread_summary_projection_preserves_ready_latest_event_id() {
        use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, OwnedEventId};
        use matrix_sdk_ui::timeline::{EmbeddedEvent, MsgLikeContent, ThreadSummary};

        let latest_event_id =
            OwnedEventId::try_from("$latest-thread-reply:test").expect("event id");
        let summary = ThreadSummary {
            latest_event: TimelineDetails::Ready(Box::new(EmbeddedEvent {
                content: TimelineItemContent::MsgLike(MsgLikeContent::redacted()),
                sender: OwnedUserId::try_from("@latest:test").expect("user id"),
                sender_profile: TimelineDetails::Unavailable,
                timestamp: MilliSecondsSinceUnixEpoch(uint!(42)),
                identifier: TimelineEventItemId::EventId(latest_event_id.clone()),
            })),
            num_replies: 1,
            public_read_receipt_event_id: None,
            private_read_receipt_event_id: None,
        };

        let dto = thread_summary_from_sdk(summary);

        assert_eq!(
            dto.latest_event_id.as_deref(),
            Some(latest_event_id.as_str())
        );
    }

    #[test]
    fn encrypted_thread_reply_relation_is_recovered_from_original_json() {
        let original_json = serde_json::json!({
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "ciphertext",
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": "$thread-root:test",
                    "m.in_reply_to": {
                        "event_id": "$reply-target:test"
                    },
                    "is_falling_back": true
                },
                "session_id": "session"
            },
            "event_id": "$thread-reply:test",
            "type": "m.room.encrypted"
        });

        assert_eq!(
            thread_root_from_original_json(&original_json).as_deref(),
            Some("$thread-root:test")
        );
    }

    #[test]
    fn megolm_session_fingerprint_is_stable_compact_and_distinguishes_rotation() {
        let first = megolm_session_fingerprint("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789");
        let same = megolm_session_fingerprint("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789");
        let rotated = megolm_session_fingerprint("ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210");

        assert_eq!(first, "AbCdEfGhIjKl");
        assert_eq!(first, same);
        assert_ne!(first, rotated);
    }

    #[test]
    fn room_timeline_keeps_renderable_thread_messages_visible() {
        let key = room_key();

        assert!(!timeline_item_should_be_hidden_for_key(
            &key,
            true,
            false,
            Some("$thread-root:test")
        ));
    }

    #[test]
    fn thread_root_activity_requires_shared_attention_eligibility() {
        let mut item = timeline_item("$reply:test", Some("reply"), "@alice:test", false);
        item.thread_root = Some("$root:test".to_owned());
        item.is_redacted = true;
        assert!(thread_root_projection_activity_from_item("!r:test", &item).is_none());

        item.is_redacted = false;
        item.is_hidden = true;
        assert!(thread_root_projection_activity_from_item("!r:test", &item).is_none());
    }

    #[test]
    fn thread_attention_does_not_count_root_or_hydrated_history_pushed_back() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let items = vec![
            timeline_message_item("$root:test", "@alice:test"),
            thread_reply_item("$historical:test", "@bob:test", "$root:test"),
        ];
        let tracker = ThreadAttentionTracker::hydrate(
            &key,
            &items,
            Some(own_user_id),
            Some("$historical:test".to_owned()),
        );

        assert_eq!(tracker.counts, ThreadAttentionCounters::default());
    }

    #[test]
    fn thread_attention_hydration_uses_visible_authoritative_receipt_baseline() {
        let key = thread_key();
        let items = vec![
            thread_reply_item("$read:test", "@alice:test", "$root:test"),
            thread_reply_item("$unread:test", "@bob:test", "$root:test"),
        ];

        let tracker = ThreadAttentionTracker::hydrate(
            &key,
            &items,
            Some("@me:test"),
            Some("$read:test".to_owned()),
        );

        assert_eq!(tracker.counts.notification_count, 1);
        assert_eq!(tracker.counts.live_event_marker_count, 1);
    }

    #[test]
    fn thread_attention_prunes_redacted_reply_before_replay() {
        let key = thread_key();
        let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some("@me:test"), None);
        let live = thread_reply_item("$live-redaction:test", "@bob:test", "$root:test");
        assert!(
            tracker
                .reconcile(
                    &key,
                    std::slice::from_ref(&live),
                    Some("@me:test"),
                    ThreadAttentionObservation::Live,
                )
                .is_some()
        );
        assert_eq!(tracker.counts.notification_count, 1);

        let mut redacted = live.clone();
        redacted.is_redacted = true;
        let provenance = ThreadAttentionBatchProvenance::from_timeline_items(
            std::slice::from_ref(&redacted),
            ThreadAttentionObservation::Replay,
        );
        assert_eq!(
            tracker.reconcile_batch(
                &key,
                std::slice::from_ref(&redacted),
                Some("@me:test"),
                &provenance,
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 0,
                highlight_count: 0,
                live_event_marker_count: 0,
            })
        );
        assert_eq!(tracker.counts.notification_count, 0);
        assert_eq!(
            tracker.reconcile(
                &key,
                std::slice::from_ref(&redacted),
                Some("@me:test"),
                ThreadAttentionObservation::Replay,
            ),
            None
        );
    }

    #[test]
    fn thread_attention_acknowledge_prunes_hidden_reply_without_reconcile() {
        let key = thread_key();
        let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some("@me:test"), None);
        let live = thread_reply_item("$live-hidden:test", "@bob:test", "$root:test");
        assert!(
            tracker
                .reconcile(
                    &key,
                    std::slice::from_ref(&live),
                    Some("@me:test"),
                    ThreadAttentionObservation::Live,
                )
                .is_some()
        );
        let mut hidden = live;
        hidden.is_hidden = true;

        assert_eq!(
            tracker.acknowledge(
                &key,
                std::slice::from_ref(&hidden),
                "$outside:test".to_owned()
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 0,
                highlight_count: 0,
                live_event_marker_count: 0,
            })
        );
    }

    #[test]
    fn thread_attention_counts_one_live_remote_reply_and_deduplicates_replay() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let mut items = vec![thread_reply_item(
            "$baseline:test",
            "@alice:test",
            "$root:test",
        )];
        let mut tracker = ThreadAttentionTracker::hydrate(
            &key,
            &items,
            Some(own_user_id),
            Some("$baseline:test".to_owned()),
        );

        let mut local_echo = thread_reply_item("$unused:test", own_user_id, "$root:test");
        local_echo.id = TimelineItemId::Transaction {
            transaction_id: "txn-own".to_owned(),
        };
        items.extend([
            local_echo,
            thread_reply_item("$own-remote:test", own_user_id, "$root:test"),
            thread_reply_item("$live:test", "@bob:test", "$root:test"),
        ]);

        assert_eq!(
            tracker.reconcile(
                &key,
                &items,
                Some(own_user_id),
                ThreadAttentionObservation::Live,
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 1,
                highlight_count: 0,
                live_event_marker_count: 1,
            })
        );
        assert_eq!(
            tracker.reconcile(
                &key,
                &items,
                Some(own_user_id),
                ThreadAttentionObservation::Replay,
            ),
            None,
            "the same stable event must not increment after reconnect/replay"
        );
        assert_eq!(tracker.counts.notification_count, 1);
    }

    #[test]
    fn live_encrypted_reply_counts_when_a_later_set_becomes_renderable() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let mut unavailable = thread_reply_item("$encrypted-live:test", "@bob:test", "$root:test");
        unavailable.body = None;
        unavailable.media = None;
        let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);

        let unavailable_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
            std::slice::from_ref(&unavailable),
            ThreadAttentionObservation::Live,
        );
        assert_eq!(
            tracker.reconcile_batch(
                &key,
                std::slice::from_ref(&unavailable),
                Some(own_user_id),
                &unavailable_provenance,
            ),
            None
        );

        let unrelated = thread_reply_item("$unrelated:test", "@alice:test", "$other-root:test");
        let unrelated_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
            std::slice::from_ref(&unrelated),
            ThreadAttentionObservation::Live,
        );
        assert_eq!(
            tracker.reconcile_batch(
                &key,
                &[unavailable, unrelated],
                Some(own_user_id),
                &unrelated_provenance,
            ),
            None,
            "an unrelated batch must not absorb the pending live encrypted event"
        );

        let renderable = thread_reply_item("$encrypted-live:test", "@bob:test", "$root:test");
        let renderable_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
            std::slice::from_ref(&renderable),
            ThreadAttentionObservation::Live,
        );
        assert_eq!(
            tracker.reconcile_batch(
                &key,
                &[renderable],
                Some(own_user_id),
                &renderable_provenance,
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 1,
                highlight_count: 0,
                live_event_marker_count: 1,
            })
        );
    }

    #[test]
    fn thread_attention_backfill_reset_and_other_roots_do_not_increment() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);
        let other_root = thread_reply_item("$other:test", "@alice:test", "$other-root:test");
        let historical = thread_reply_item("$old:test", "@bob:test", "$root:test");

        assert_eq!(
            tracker.reconcile(
                &key,
                std::slice::from_ref(&historical),
                Some(own_user_id),
                ThreadAttentionObservation::Backfill,
            ),
            None
        );
        assert_eq!(
            tracker.reconcile(
                &key,
                &[historical, other_root],
                Some(own_user_id),
                ThreadAttentionObservation::Replay,
            ),
            None
        );
        assert_eq!(tracker.counts, ThreadAttentionCounters::default());

        let receipt = thread_reply_item("$visible-read:test", own_user_id, "$root:test");
        let after_receipt = thread_reply_item("$historical-after:test", "@bob:test", "$root:test");
        let mut tracker = ThreadAttentionTracker::hydrate(
            &key,
            std::slice::from_ref(&receipt),
            Some(own_user_id),
            Some("$visible-read:test".to_owned()),
        );
        assert_eq!(
            tracker.reconcile(
                &key,
                &[receipt, after_receipt],
                Some(own_user_id),
                ThreadAttentionObservation::Backfill,
            ),
            None,
            "ordinary pagination never manufactures attention"
        );
        assert_eq!(tracker.counts, ThreadAttentionCounters::default());
    }

    #[test]
    fn delayed_pagination_batch_does_not_become_live_after_task_completion() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let historical = thread_reply_item("$old-delayed:test", "@bob:test", "$root:test");
        let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);

        // Reproduce the actor race reported by independent review: the SDK
        // pagination call has completed and cleared ambient task state before
        // its separately relayed PushBack batch reaches the actor.
        let delayed_pagination_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
            std::slice::from_ref(&historical),
            ThreadAttentionObservation::Backfill,
        );

        assert_eq!(
            tracker.reconcile_batch(
                &key,
                std::slice::from_ref(&historical),
                Some(own_user_id),
                &delayed_pagination_provenance,
            ),
            None,
            "pagination provenance must travel with the delayed batch"
        );
        assert_eq!(tracker.counts, ThreadAttentionCounters::default());
    }

    #[test]
    fn sdk_event_origin_is_the_relay_batch_attention_provenance() {
        assert_eq!(
            thread_attention_observation_from_event_origin(Some(EventItemOrigin::Sync)),
            ThreadAttentionObservation::Live
        );
        assert_eq!(
            thread_attention_observation_from_event_origin(Some(EventItemOrigin::Pagination)),
            ThreadAttentionObservation::Backfill
        );
        assert_eq!(
            thread_attention_observation_from_event_origin(Some(EventItemOrigin::Cache)),
            ThreadAttentionObservation::Replay
        );
        assert_eq!(
            thread_attention_observation_from_event_origin(None),
            ThreadAttentionObservation::Replay,
            "unknown and delayed hydration must be conservative"
        );
    }

    #[test]
    fn thread_attention_trackers_do_not_contaminate_different_threads() {
        let first_key = thread_key();
        let second_key = TimelineKey {
            account_key: first_key.account_key.clone(),
            kind: TimelineKind::Thread {
                room_id: "!r:test".to_owned(),
                root_event_id: "$second-root:test".to_owned(),
            },
        };
        let first_live = thread_reply_item("$first-live:test", "@alice:test", "$root:test");
        let mut first = ThreadAttentionTracker::hydrate(&first_key, &[], Some("@me:test"), None);
        let mut second = ThreadAttentionTracker::hydrate(&second_key, &[], Some("@me:test"), None);

        assert!(
            first
                .reconcile(
                    &first_key,
                    std::slice::from_ref(&first_live),
                    Some("@me:test"),
                    ThreadAttentionObservation::Live,
                )
                .is_some()
        );
        assert_eq!(
            second.reconcile(
                &second_key,
                &[first_live],
                Some("@me:test"),
                ThreadAttentionObservation::Live,
            ),
            None
        );
        assert_eq!(first.counts.notification_count, 1);
        assert_eq!(second.counts.notification_count, 0);
    }

    #[test]
    fn thread_attention_acknowledgement_clears_without_changing_total_reply_count() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let mut root = timeline_message_item("$root:test", "@alice:test");
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 2,
            latest_event_id: Some("$live:test".to_owned()),
            latest_sender: Some("@bob:test".to_owned()),
            latest_sender_label: Some("Bob".to_owned()),
            latest_body_preview: Some("preview".to_owned()),
            latest_timestamp_ms: Some(2),
        });
        let items = vec![
            root,
            thread_reply_item("$baseline:test", "@alice:test", "$root:test"),
            thread_reply_item("$live:test", "@bob:test", "$root:test"),
        ];
        let mut tracker = ThreadAttentionTracker::hydrate(
            &key,
            &items[..2],
            Some(own_user_id),
            Some("$baseline:test".to_owned()),
        );
        let _ = tracker.reconcile(
            &key,
            &items,
            Some(own_user_id),
            ThreadAttentionObservation::Live,
        );

        assert_eq!(tracker.counts.notification_count, 1);
        assert_eq!(items[0].thread_summary.as_ref().unwrap().reply_count, 2);
        assert_eq!(
            tracker.acknowledge(&key, &items, "$outside-window:test".to_owned()),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 1,
                highlight_count: 0,
                live_event_marker_count: 1,
            }),
            "an out-of-window receipt must not guess the relative ordering"
        );
        assert_eq!(
            tracker.acknowledge(&key, &items, "$live:test".to_owned()),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 0,
                highlight_count: 0,
                live_event_marker_count: 0,
            })
        );
        assert_eq!(items[0].thread_summary.as_ref().unwrap().reply_count, 2);
    }

    #[test]
    fn visible_receipt_prunes_attention_preserved_while_it_was_outside_the_window() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let live = thread_reply_item("$live-before-receipt:test", "@bob:test", "$root:test");
        let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);
        let _ = tracker.reconcile(
            &key,
            std::slice::from_ref(&live),
            Some(own_user_id),
            ThreadAttentionObservation::Live,
        );
        assert_eq!(tracker.counts.notification_count, 1);
        let _ = tracker.acknowledge(
            &key,
            std::slice::from_ref(&live),
            "$later-receipt:test".to_owned(),
        );
        assert_eq!(tracker.counts.notification_count, 1);

        let receipt = thread_reply_item("$later-receipt:test", own_user_id, "$root:test");
        let expanded = vec![live, receipt];
        assert_eq!(
            tracker.reconcile(
                &key,
                &expanded,
                Some(own_user_id),
                ThreadAttentionObservation::Backfill,
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 0,
                highlight_count: 0,
                live_event_marker_count: 0,
            })
        );
    }

    #[test]
    fn recovery_counts_first_seen_unread_reply_after_visible_receipt() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let receipt = thread_reply_item("$read-before-overflow:test", own_user_id, "$root:test");
        let unread = thread_reply_item("$missed-during-overflow:test", "@bob:test", "$root:test");
        let mut tracker = ThreadAttentionTracker::hydrate(
            &key,
            std::slice::from_ref(&receipt),
            Some(own_user_id),
            Some("$read-before-overflow:test".to_owned()),
        );

        assert_eq!(
            tracker.reconcile(
                &key,
                &[receipt, unread],
                Some(own_user_id),
                ThreadAttentionObservation::Replay,
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 1,
                highlight_count: 0,
                live_event_marker_count: 1,
            })
        );
    }

    #[test]
    fn recovery_and_manager_owned_receipt_success_preserve_attention_ordering() {
        let recovery = item_body(include_str!("relay.rs"), "async fn handle_relay_overflow");
        assert!(
            recovery.contains("if let Some(action) = self.thread_attention.reconcile")
                && recovery.contains("self.emit_action_reliable(action).await"),
            "recovery-driven attention changes must reach the reducer"
        );
        let startup = item_body(include_str!("actor.rs"), "async fn spawn(");
        let subscribe = startup
            .find("subscribe_own_user_read_receipts_changed")
            .expect("receipt changes must be subscribed");
        let query = startup
            .find("latest_user_read_receipt_timeline_event_id")
            .expect("initial receipt must be queried");
        assert!(
            subscribe < query,
            "subscribe-before-query closes the startup receipt race"
        );
        let manager_completion = item_body(
            include_str!("read_state.rs"),
            "async fn handle_read_worker_completion",
        );
        assert!(
            manager_completion.contains("self.spawn_read_actor_apply(operation.clone())")
                && !manager_completion.contains("LiveSignalsEvent::ReadReceiptSent"),
            "threaded network success must wait for actor control-lane application before terminal success"
        );
        let actor_apply = item_body(
            include_str!("read_state.rs"),
            "async fn handle_read_success",
        );
        let acknowledge = actor_apply
            .find("thread_attention.acknowledge")
            .expect("threaded read success must acknowledge the tracker");
        let reliable_action = actor_apply
            .find("emit_action_reliable(action).await")
            .expect("thread attention acknowledgement must reach the reducer reliably");
        assert!(
            acknowledge < reliable_action,
            "thread attention must update before the actor acknowledges read success"
        );
        let settlement = item_body(
            include_str!("read_state.rs"),
            "async fn settle_read_waiters",
        );
        assert!(
            settlement.contains("LiveSignalsEvent::ReadReceiptSent"),
            "only manager settlement may emit the existing read-receipt success event"
        );
    }

    #[test]
    fn successful_receipt_uses_newest_provable_canonical_boundary() {
        let items = vec![
            thread_reply_item("$old-read:test", "@me:test", "$root:test"),
            thread_reply_item("$requested-read:test", "@me:test", "$root:test"),
            thread_reply_item("$newer-device-read:test", "@me:test", "$root:test"),
        ];

        let requested = "$requested-read:test";
        let selected = newest_provable_receipt_event_id(
            &items,
            requested,
            Some("$old-read:test".to_owned()),
            Some("$old-read:test"),
        );
        assert_eq!(
            selected, requested,
            "a stale SDK query must not delay the successful newer request"
        );

        assert_eq!(
            newest_provable_receipt_event_id(
                &items,
                "$requested-read:test",
                Some("$old-read:test".to_owned()),
                Some("$newer-device-read:test"),
            ),
            "$newer-device-read:test",
            "a stale request must not regress a newer multi-device boundary"
        );

        assert_eq!(
            newest_provable_receipt_event_id(
                &items[1..2],
                "$requested-read:test",
                Some("$queried-outside-window:test".to_owned()),
                Some("$current-outside-window:test"),
            ),
            "$requested-read:test",
            "unknown out-of-window IDs cannot override a visible successful request"
        );
    }
}
