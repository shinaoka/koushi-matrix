use crate::{
    executor,
    link_preview::LinkPreviewContext,
    timeline::{
        actor::{TimelineActor, TimelineActorMessage},
        manager::{TimelineManagerControl, TimelineMessage},
        outbound_send::{PendingSendPhase, PendingSendProjection, pending_send_item},
        relay::koushi_timeline_builder,
        test_support::{fake_rid, live_tail_test_manager},
    },
};
use koushi_protocol::{
    command::TimelineCommand,
    event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{event_id, room_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, User, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast, mpsc, oneshot};

pub(super) const TARGET: &str = "$mutation:example.invalid";
pub(super) const REACTION: &str = "$reaction:example.invalid";
pub(super) const PENDING: &str = "synthetic-pending";
pub(super) const LIVENESS: Duration = Duration::from_secs(10);

pub(super) struct Fixture {
    pub server: MatrixMockServer,
    pub key: TimelineKey,
    pub generation: u64,
    pub events: broadcast::Receiver<CoreEvent>,
    pub event_tx: broadcast::Sender<CoreEvent>,
    pub context: crate::timeline::mutation::MutationContext,
    pub coordinator: crate::timeline::outbound_send::SharedSendCompletionCoordinator,
    pub terminal_ingress: crate::timeline::outbound_send::TimelineSendTerminalIngress,
    pending_registration: Option<crate::timeline::outbound_send::SendCompletionRegistration>,
    pub actor: mpsc::Sender<TimelineActorMessage>,
    pub manager: mpsc::Sender<TimelineMessage>,
    control: mpsc::Sender<TimelineManagerControl>,
    task: Option<executor::JoinHandle<()>>,
    drain: Option<executor::JoinHandle<()>>,
    /// Detached abort handle for the actor task, so a test can stop the
    /// presentation actor from polling without owning its `JoinHandle`.
    actor_abort: tokio::task::AbortHandle,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
        if let Some(task) = &self.drain {
            task.abort();
        }
    }
}
impl Fixture {
    pub async fn new(edit: bool) -> Self {
        Self::with_deadline(edit, None).await
    }
    pub async fn with_deadline(edit: bool, deadline: Option<Duration>) -> Self {
        let server = MatrixMockServer::new().await;
        let client = server
            .client_builder()
            .logged_in_with_token("synthetic".into(), User::Alice.into(), "SYNTHETIC".into())
            .build()
            .await;
        let room_id = room_id!("!mutation:example.invalid");
        server
            .mock_room_state_encryption()
            .expect_access_token("synthetic")
            .plain()
            .mount()
            .await;
        let room = server.sync_joined_room(&client, room_id).await;
        let factory = EventFactory::new().room(room_id).sender(User::Alice);
        let event = factory
            .text_msg("Synthetic original")
            .event_id(event_id!("$mutation:example.invalid"));
        let (key, focus) = if edit {
            use wiremock::{
                Mock, ResponseTemplate,
                matchers::{method, path_regex},
            };
            Mock::given(method("GET")).and(path_regex("/context/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event": event.into_event().into_raw().json(), "events_before": [], "events_after": [], "state": []
                }))).mount(server.server()).await;
            (
                TimelineKey {
                    account_key: AccountKey(User::Alice.to_string()),
                    kind: koushi_protocol::ids::TimelineKind::Focused {
                        room_id: room_id.to_string(),
                        event_id: TARGET.to_owned(),
                    },
                },
                TimelineFocus::Event {
                    target: event_id!("$mutation:example.invalid").to_owned(),
                    num_context_events: 0,
                    thread_mode: matrix_sdk_ui::timeline::TimelineEventFocusThreadMode::Automatic {
                        hide_threaded_events: false,
                    },
                },
            )
        } else {
            server
                .sync_room(
                    &client,
                    JoinedRoomBuilder::new(room_id)
                        .add_timeline_event(event.into_raw_sync())
                        .add_timeline_event(
                            factory
                                .reaction(event_id!("$mutation:example.invalid"), "👍")
                                .event_id(event_id!("$reaction:example.invalid"))
                                .into_raw_sync(),
                        ),
                )
                .await;
            (
                TimelineKey::room(AccountKey(User::Alice.to_string()), room_id.to_string()),
                TimelineFocus::Live {
                    hide_threaded_events: false,
                },
            )
        };
        let timeline = Arc::new(koushi_timeline_builder(&room, focus).build().await.unwrap());
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.uri(),
                user_id: User::Alice.to_string(),
                device_id: "SYNTHETIC".into(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let mut manager = live_tail_test_manager(Default::default());
        if let Some(deadline) = deadline {
            manager.mutation_lane =
                crate::timeline::mutation::MutationLane::with_deadline(deadline);
        }

        let (actions, mut rx) = mpsc::channel(64);
        manager.action_tx = actions;
        let drain = executor::spawn(async move { while rx.recv().await.is_some() {} });
        manager.event_tx = broadcast::channel(256).0;
        let event_tx = manager.event_tx.clone();
        let events = event_tx.subscribe();
        let generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let mutation_timeline = timeline.clone();
        let handle = TimelineActor::spawn(
            key.clone(),
            timeline,
            session,
            fake_rid(1),
            true,
            manager.action_tx.clone(),
            manager.event_tx.clone(),
            None,
            Default::default(),
            None,
            LinkPreviewContext::default(),
            manager.account_work.clone(),
            manager.thread_root_projection_service.clone(),
            manager.thread_root_order,
            false,
            manager.timeline_actor_generations.clone(),
            generation,
            None,
            Default::default(),
            manager.terminal_ingress.clone(),
            manager.msg_tx.clone(),
        )
        .await;
        let sent_event_txns = match handle.enqueue_context.as_ref().unwrap() {
            crate::timeline::outbound_send::TimelineSendEnqueueContext::Matrix(context) => {
                context.sent_event_txns.clone()
            }
            _ => unreachable!(),
        };
        let context = crate::timeline::mutation::MutationContext {
            key: key.clone(),
            generation,
            timeline: mutation_timeline,
            gate: manager.timeline_actor_generations.clone(),
            account_work: manager.account_work.clone(),
            sent_event_txns,
        };
        let actor = handle.tx.clone();
        let actor_abort = handle
            .task
            .as_ref()
            .expect("actor task is spawned")
            .abort_handle();
        manager.timelines.insert(key.clone(), handle);
        let coordinator = manager.send_completion.clone();
        let terminal_ingress = manager.terminal_ingress.clone();
        let sender = manager.msg_tx.clone();
        let (control, control_rx) = mpsc::channel(4);
        manager.control_rx = Some(control_rx);
        let task = executor::spawn(manager.run());
        let mut this = Self {
            server,
            key,
            generation,
            events,
            event_tx,
            context,
            coordinator,
            terminal_ingress,
            pending_registration: None,
            actor,
            manager: sender,
            control,
            task: Some(task),
            drain: Some(drain),
            actor_abort,
        };
        this.wait_for(|event| {
            if let CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) = event {
                let target = items.iter().find(|item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == TARGET)).expect("target present");
                if !edit { assert!(target.can_react, "target permits reactions"); assert!(target.reactions.iter().any(|reaction| reaction.my_reaction_event_id.as_deref() == Some(REACTION)), "own remote reaction present: {:?}", target.reactions); }
                true
            } else { false }
        }).await;
        this.actor
            .send(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: None,
            })
            .await
            .unwrap();
        this.wait_for(|event| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
            )
        })
        .await;
        this
    }
    pub async fn command(&self, command: TimelineCommand) {
        self.manager
            .send(TimelineMessage::Command(command))
            .await
            .unwrap();
    }
    /// Stop the presentation actor from polling, so its mailbox never accepts.
    pub fn stop_actor(&self) {
        self.actor_abort.abort();
    }
    pub async fn publish(&self, projections: Vec<PendingSendProjection>) -> bool {
        let (acknowledged, accepted) = oneshot::channel();
        self.actor
            .send(TimelineActorMessage::RefreshPendingSendProjection {
                actor_generation: self.generation,
                acknowledged,
                projections,
            })
            .await
            .unwrap();
        tokio::time::timeout(LIVENESS, accepted)
            .await
            .expect("actor handles pending refresh")
            .unwrap()
    }
    pub fn projection(&self) -> PendingSendProjection {
        PendingSendProjection {
            lifecycle_trace: None,
            key: self.key.clone(),
            sequence: 1,
            client_txn_id: PENDING.into(),
            item: pending_send_item(
                PENDING,
                "Synthetic pending",
                None,
                None,
                Some(User::Alice.as_str()),
            ),
            sdk_transaction_id: None,
            handle: None,
            terminal_event_id: None,
            phase: PendingSendPhase::Pending,
        }
    }
    pub async fn pending(&mut self) {
        let mut registration =
            crate::timeline::outbound_send::SendCompletionRegistration::begin_with_projection(
                self.coordinator.clone(),
                self.terminal_ingress.clone(),
                self.key.clone(),
                PENDING.into(),
                None,
                fake_rid(700),
                true,
                Some(self.projection()),
            );
        registration.activate();
        registration.bind(PENDING.into());
        self.pending_registration = Some(registration);
        let projections = self
            .coordinator
            .lock()
            .unwrap()
            .projections_for_key(&self.key);
        assert!(self.publish(projections).await);
    }
    pub async fn wait_for(&mut self, predicate: impl Fn(&CoreEvent) -> bool) {
        self.wait_for_stage("actor_publication", predicate).await;
    }
    pub async fn wait_for_stage(&mut self, stage: &str, predicate: impl Fn(&CoreEvent) -> bool) {
        tokio::time::timeout(LIVENESS, async {
            loop {
                let event = self.events.recv().await.unwrap();
                if predicate(&event) {
                    return;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("publication stage {stage} stalled while HTTP remains held"));
    }
    pub async fn stop(mut self) {
        let (acknowledged, ack) = oneshot::channel();
        self.control
            .send(TimelineManagerControl::Shutdown { acknowledged })
            .await
            .unwrap();
        tokio::time::timeout(LIVENESS, ack).await.unwrap().unwrap();
        self.task.take().unwrap().await.unwrap();
        let task = self.drain.take().unwrap();
        task.abort();
        let _ = task.await;
    }
}
pub(super) fn is_pending(event: &CoreEvent) -> bool {
    matches!(event, CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) if diffs.iter().any(|diff| {
        let item: Option<&TimelineItem> = match diff {
            koushi_protocol::event::TimelineDiff::PushFront { item } | koushi_protocol::event::TimelineDiff::PushBack { item } | koushi_protocol::event::TimelineDiff::Insert { item, .. } | koushi_protocol::event::TimelineDiff::Set { item, .. } => Some(item),
            koushi_protocol::event::TimelineDiff::Reset { items } => return items.iter().any(|item| matches!(&item.id, TimelineItemId::Transaction { transaction_id } if transaction_id == PENDING)),
            _ => None,
        };
        item.is_some_and(|item| matches!(&item.id, TimelineItemId::Transaction { transaction_id } if transaction_id == PENDING))
    }))
}
