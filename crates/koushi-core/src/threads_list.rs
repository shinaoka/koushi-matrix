//! ThreadsListActor: per-room thread list subscription and pagination.
//!
//! Wraps the SDK `ThreadListService` and projects `ThreadListItem`s into the
//! app-owned `ThreadsListItem` DTO. All state transitions are delivered as
//! typed `AppAction`s (and mirrored as `CoreEvent::ThreadsList` events) so the
//! reducer owns the UI snapshot.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures_util::StreamExt;
use koushi_state::{AppAction, OperationFailureKind, ThreadsListItem};
use matrix_sdk::ruma::RoomId;
use matrix_sdk_ui::timeline::thread_list_service::{
    ThreadListItem as SdkThreadListItem, ThreadListServiceError,
};
use matrix_sdk_ui::timeline::{ThreadListPaginationState, ThreadListService, TimelineDetails};
use tokio::sync::{broadcast, mpsc};

use crate::event::{CoreEvent, ThreadsListEvent, TimelineItem};
use crate::executor;
use crate::ids::RequestId;

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
        match self.attempt {
            ThreadRootProjectionAttempt::Failed(kind) => Some(kind),
            ThreadRootProjectionAttempt::Pending | ThreadRootProjectionAttempt::Ready(_) => None,
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        matches!(self.attempt, ThreadRootProjectionAttempt::Pending)
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
            if activity_is_newer(&activity, &record.activity) {
                // A failed root stays terminal while its reply remains in the
                // active Room window. We still advance its activity identity
                // so the unavailable placeholder follows the latest reply,
                // but must never turn it into another fetch attempt.
                record.activity = activity;
                return ThreadRootProjectionDecision::ActivityUpdated(record.clone());
            }
            return ThreadRootProjectionDecision::Existing(record.clone());
        }
        self.attempts.insert(
            key,
            ThreadRootProjectionRecord {
                activity: activity.clone(),
                attempt: ThreadRootProjectionAttempt::Pending,
            },
        );
        ThreadRootProjectionDecision::StartFetch(activity)
    }

    /// Keep only projection data that still has a representation in the
    /// bounded canonical Room window. Pending requests are retained until
    /// their one worker completes; terminal records are dropped as soon as the
    /// corresponding root has no live reply. Thus a reconnect can dedupe a
    /// currently-active failure, while a later observation after cleanup is a
    /// new bounded attempt rather than a retry loop.
    pub(crate) fn reconcile_room(
        &mut self,
        room_id: &str,
        active_root_event_ids: &HashSet<String>,
    ) {
        self.active_root_event_ids
            .insert(room_id.to_owned(), active_root_event_ids.clone());
        self.attempts
            .retain(|(entry_room_id, root_event_id), record| {
                entry_room_id != room_id
                    || active_root_event_ids.contains(root_event_id)
                    || record.is_pending()
            });
        self.cleanup_empty_room_tracking(room_id);
    }

    /// Reconcile the Room's current selected reply for every root. Unlike
    /// [`Self::observe`], this may move a retained terminal record backwards:
    /// a newer reply can leave the bounded SDK window while an older reply for
    /// the same root remains. The bounded lookup is still exactly-once because
    /// only the activity metadata changes; the attempt state is preserved.
    pub(crate) fn reconcile_room_activities(
        &mut self,
        room_id: &str,
        activities_by_root: &HashMap<String, ThreadRootProjectionActivity>,
    ) {
        let active_root_event_ids = activities_by_root.keys().cloned().collect::<HashSet<_>>();
        self.reconcile_room(room_id, &active_root_event_ids);
        for (root_event_id, activity) in activities_by_root {
            if let Some(record) = self
                .attempts
                .get_mut(&(room_id.to_owned(), root_event_id.clone()))
            {
                record.activity = activity.clone();
            }
        }
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
            .is_some_and(ThreadRootProjectionRecord::is_pending)
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

pub(crate) fn activity_is_newer(
    candidate: &ThreadRootProjectionActivity,
    existing: &ThreadRootProjectionActivity,
) -> bool {
    candidate
        .activity_timestamp_ms
        .unwrap_or(0)
        .cmp(&existing.activity_timestamp_ms.unwrap_or(0))
        .then_with(|| candidate.activity_event_id.cmp(&existing.activity_event_id))
        .is_gt()
}

/// Messages routed to a `ThreadsListActor`.
pub enum ThreadsListMessage {
    Open { request_id: RequestId },
    Close { request_id: RequestId },
    Paginate { request_id: RequestId },
    Shutdown,
}

/// Handle to a `ThreadsListActor` background task.
pub struct ThreadsListActorHandle {
    tx: mpsc::Sender<ThreadsListMessage>,
    room_id: String,
}

impl ThreadsListActorHandle {
    pub async fn open(&self, _request_id: RequestId, room_id: String) -> bool {
        // The handle is keyed to a room; the caller already verified the room id.
        let _ = room_id;
        self.tx
            .send(ThreadsListMessage::Open {
                request_id: _request_id,
            })
            .await
            .is_ok()
    }

    pub async fn close(&self, request_id: RequestId) -> bool {
        self.tx
            .send(ThreadsListMessage::Close { request_id })
            .await
            .is_ok()
    }

    pub async fn paginate(&self, request_id: RequestId) -> bool {
        self.tx
            .send(ThreadsListMessage::Paginate { request_id })
            .await
            .is_ok()
    }

    pub fn room_id(&self) -> &str {
        self.room_id.as_str()
    }
}

pub struct ThreadsListActor {
    session: Arc<koushi_sdk::MatrixClientSession>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    room_id: String,
    msg_rx: mpsc::Receiver<ThreadsListMessage>,
}

impl ThreadsListActor {
    pub fn spawn(
        session: Arc<koushi_sdk::MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        room_id: String,
    ) -> ThreadsListActorHandle {
        let (tx, msg_rx) = mpsc::channel(16);
        let actor = ThreadsListActor {
            session,
            action_tx,
            event_tx,
            room_id: room_id.clone(),
            msg_rx,
        };
        executor::spawn(actor.run());
        ThreadsListActorHandle { tx, room_id }
    }

    async fn run(mut self) {
        let mut active: Option<ActiveSubscription> = None;
        while let Some(msg) = self.msg_rx.recv().await {
            match msg {
                ThreadsListMessage::Shutdown | ThreadsListMessage::Close { .. } => {
                    active = None;
                    if matches!(msg, ThreadsListMessage::Shutdown) {
                        break;
                    }
                }
                ThreadsListMessage::Open { request_id } => {
                    active = self.open_subscription(request_id).await;
                }
                ThreadsListMessage::Paginate { request_id } => {
                    if let Some(sub) = active.as_ref() {
                        sub.paginate(request_id).await;
                    }
                }
            }
        }
        // Dropping `active` cancels the SDK subscription background tasks.
    }

    async fn open_subscription(&self, request_id: RequestId) -> Option<ActiveSubscription> {
        let room_id = match RoomId::parse(self.room_id.as_str()) {
            Ok(id) => id,
            Err(_) => {
                self.emit_failed(request_id, OperationFailureKind::Invalid)
                    .await;
                return None;
            }
        };
        let room = match self.session.client().get_room(&room_id) {
            Some(room) => room,
            None => {
                self.emit_failed(request_id, OperationFailureKind::NotFound)
                    .await;
                return None;
            }
        };

        let service = Arc::new(ThreadListService::new(room));
        let (_, items_subscriber) = service.subscribe_to_items_updates();
        if let Err(_) = service.paginate().await {
            self.emit_failed(request_id, OperationFailureKind::Sdk)
                .await;
            return None;
        }
        let items = service.items();
        let projected: Vec<ThreadsListItem> = items.iter().map(project_item).collect();
        let end_reached = matches!(
            service.pagination_state(),
            ThreadListPaginationState::Idle { end_reached: true }
        );

        self.emit_opened(request_id, projected.clone(), end_reached)
            .await;

        let (items_tx, mut items_rx) = mpsc::channel(64);
        let (pagination_tx, mut pagination_rx) = mpsc::channel(16);
        let (pagination_request_tx, mut pagination_request_rx) = mpsc::channel(16);
        let (pagination_failure_tx, mut pagination_failure_rx) = mpsc::channel(16);

        let items_relay_handle = {
            let service = Arc::clone(&service);
            let mut subscriber = items_subscriber;
            executor::spawn(async move {
                loop {
                    match subscriber.next().await {
                        Some(_) => {
                            if items_tx.send(service.items()).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            })
        };

        let pagination_relay_handle = {
            let service = Arc::clone(&service);
            let mut subscriber = service.subscribe_to_pagination_state_updates();
            executor::spawn(async move {
                loop {
                    match subscriber.next().await {
                        Some(state) => {
                            if pagination_tx.send(state).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            })
        };

        let action_tx = self.action_tx.clone();
        let event_tx = self.event_tx.clone();
        let room_id = self.room_id.clone();
        let update_service = Arc::clone(&service);
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
                        let _ = action_tx.send(vec![AppAction::ThreadsListFailed {
                            request_id: failed_request_id.sequence,
                            room_id: room_id.clone(),
                            failure_kind,
                        }]).await;
                        let _ = event_tx.send(CoreEvent::ThreadsList(ThreadsListEvent::Failed {
                            request_id: failed_request_id,
                            room_id: room_id.clone(),
                            failure_kind,
                        }));
                    }
                    Some(items) = items_rx.recv() => {
                        let projected: Vec<ThreadsListItem> = items.iter().map(project_item).collect();
                        let _ = action_tx.send(vec![AppAction::ThreadsListUpdated {
                            request_id: current_request_id.sequence,
                            room_id: room_id.clone(),
                            items: projected.clone(),
                            is_paginating: false,
                            end_reached: false,
                        }]).await;
                        let _ = event_tx.send(CoreEvent::ThreadsList(ThreadsListEvent::Updated {
                            request_id: current_request_id,
                            room_id: room_id.clone(),
                            items: projected,
                            is_paginating: false,
                            end_reached: false,
                        }));
                    }
                    Some(state) = pagination_rx.recv() => {
                        let items = update_service.items();
                        let projected: Vec<ThreadsListItem> = items.iter().map(project_item).collect();
                        let end_reached = matches!(state, ThreadListPaginationState::Idle { end_reached: true });
                        let is_paginating = matches!(state, ThreadListPaginationState::Loading);
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
                                room_id: room_id.clone(),
                                items: projected.clone(),
                                is_paginating: true,
                                end_reached,
                            }
                        } else {
                            AppAction::ThreadsListPaginationCompleted {
                                request_id: current_request_id.sequence,
                                room_id: room_id.clone(),
                                items: projected.clone(),
                                end_reached,
                            }
                        };
                        let _ = action_tx.send(vec![action]).await;
                        let event = if is_paginating {
                            CoreEvent::ThreadsList(ThreadsListEvent::Updated {
                                request_id: current_request_id,
                                room_id: room_id.clone(),
                                items: projected.clone(),
                                is_paginating: true,
                                end_reached,
                            })
                        } else {
                            CoreEvent::ThreadsList(ThreadsListEvent::PaginationCompleted {
                                request_id: current_request_id,
                                room_id: room_id.clone(),
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

        Some(ActiveSubscription {
            service,
            pagination_request_tx,
            pagination_failure_tx,
            _items_relay: items_relay_handle,
            _pagination_relay: pagination_relay_handle,
            _update_task: update_task,
        })
    }

    async fn emit_opened(
        &self,
        request_id: RequestId,
        items: Vec<ThreadsListItem>,
        end_reached: bool,
    ) {
        let room_id = self.room_id.clone();
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

    async fn emit_failed(&self, request_id: RequestId, failure_kind: OperationFailureKind) {
        let room_id = self.room_id.clone();
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
    service: Arc<ThreadListService>,
    pagination_request_tx: mpsc::Sender<RequestId>,
    pagination_failure_tx: mpsc::Sender<(RequestId, OperationFailureKind)>,
    _items_relay: executor::JoinHandle<()>,
    _pagination_relay: executor::JoinHandle<()>,
    _update_task: executor::JoinHandle<()>,
}

impl ActiveSubscription {
    async fn paginate(&self, request_id: RequestId) {
        if self.pagination_request_tx.send(request_id).await.is_err() {
            return;
        }
        if let Err(error) = self.service.paginate().await {
            let failure_kind = classify_thread_list_error(&error);
            let _ = self
                .pagination_failure_tx
                .send((request_id, failure_kind))
                .await;
        }
    }
}

fn classify_thread_list_error(error: &ThreadListServiceError) -> OperationFailureKind {
    match error {
        ThreadListServiceError::Sdk(matrix_sdk::Error::Http(_)) => OperationFailureKind::Network,
        ThreadListServiceError::Sdk(_) => OperationFailureKind::Sdk,
    }
}

fn project_item(item: &SdkThreadListItem) -> ThreadsListItem {
    ThreadsListItem {
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    use crate::event::{TimelineItem, TimelineItemId, TimelineMessageActions};

    use super::{
        OperationFailureKind, ThreadRootProjectionActivity, ThreadRootProjectionDecision,
        ThreadRootProjectionService,
    };

    fn test_timeline_item(event_id: &str) -> TimelineItem {
        TimelineItem {
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
            .split("Some(state) = pagination_rx.recv()")
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
            .split("#[cfg(test)]")
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
            open_subscription.contains("items_tx.send(service.items()).await"),
            "item relay should await reliable delivery to the update task"
        );
        assert!(
            open_subscription.contains("pagination_tx.send(state).await"),
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
                .contains("self.emit_failed(request_id, OperationFailureKind::Invalid)")
                && open_subscription
                    .contains("self.emit_failed(request_id, OperationFailureKind::NotFound)"),
            "open failures should preserve parse/not-found failure kinds"
        );
    }
}
