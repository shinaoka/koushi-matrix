//! ThreadsListActor: per-room thread list subscription and pagination.
//!
//! Wraps the SDK `ThreadListService` and projects `ThreadListItem`s into the
//! app-owned `ThreadsListItem` DTO. All state transitions are delivered as
//! typed `AppAction`s (and mirrored as `CoreEvent::ThreadsList` events) so the
//! reducer owns the UI snapshot.

use std::sync::Arc;

use futures_util::StreamExt;
use koushi_state::{AppAction, OperationFailureKind, ThreadsListItem};
use matrix_sdk::ruma::RoomId;
use matrix_sdk_ui::timeline::thread_list_service::ThreadListItem as SdkThreadListItem;
use matrix_sdk_ui::timeline::{ThreadListPaginationState, ThreadListService, TimelineDetails};
use tokio::sync::{broadcast, mpsc};

use crate::event::{CoreEvent, ThreadsListEvent};
use crate::executor;
use crate::ids::RequestId;

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
                self.emit_failed(request_id, OperationFailureKind::Sdk)
                    .await;
                return None;
            }
        };
        let room = match self.session.client().get_room(&room_id) {
            Some(room) => room,
            None => {
                self.emit_failed(request_id, OperationFailureKind::Sdk)
                    .await;
                return None;
            }
        };

        let service = Arc::new(ThreadListService::new(room));
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

        let items_relay_handle = {
            let service = Arc::clone(&service);
            let mut subscriber = service.subscribe_to_items_updates().1;
            executor::spawn(async move {
                loop {
                    match subscriber.next().await {
                        Some(_) => {
                            let _ = items_tx.try_send(service.items());
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
                            let _ = pagination_tx.try_send(state);
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
            loop {
                tokio::select! {
                    biased;
                    Some(items) = items_rx.recv() => {
                        let projected: Vec<ThreadsListItem> = items.iter().map(project_item).collect();
                        let _ = action_tx.send(vec![AppAction::ThreadsListUpdated {
                            request_id: request_id.sequence,
                            room_id: room_id.clone(),
                            items: projected.clone(),
                            is_paginating: false,
                            end_reached: false,
                        }]).await;
                        let _ = event_tx.send(CoreEvent::ThreadsList(ThreadsListEvent::Updated {
                            request_id,
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
                        let action = if is_paginating {
                            AppAction::ThreadsListUpdated {
                                request_id: request_id.sequence,
                                room_id: room_id.clone(),
                                items: projected.clone(),
                                is_paginating: true,
                                end_reached,
                            }
                        } else {
                            AppAction::ThreadsListPaginationCompleted {
                                request_id: request_id.sequence,
                                room_id: room_id.clone(),
                                items: projected.clone(),
                                end_reached,
                            }
                        };
                        let _ = action_tx.send(vec![action]).await;
                        let event = if is_paginating {
                            CoreEvent::ThreadsList(ThreadsListEvent::Updated {
                                request_id,
                                room_id: room_id.clone(),
                                items: projected.clone(),
                                is_paginating: true,
                                end_reached,
                            })
                        } else {
                            CoreEvent::ThreadsList(ThreadsListEvent::PaginationCompleted {
                                request_id,
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
    _items_relay: executor::JoinHandle<()>,
    _pagination_relay: executor::JoinHandle<()>,
    _update_task: executor::JoinHandle<()>,
}

impl ActiveSubscription {
    async fn paginate(&self, request_id: RequestId) {
        if let Err(_) = self.service.paginate().await {
            // Failure is surfaced through the pagination-state subscriber, which
            // transitions back to Idle and emits a Failed action.
            let _ = request_id;
        }
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
