//! Shared cfg-test-only fixtures used by multiple timeline owner suites.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use futures_util::{FutureExt, StreamExt};
use koushi_state::ComposerFormattingOptions;

use tokio::sync::{broadcast, mpsc};

use crate::account_work::AccountWorkScheduler;
use crate::event::{
    TimelineItem, TimelineItemId, TimelineMedia, TimelineMediaKind, TimelineMediaSource,
    TimelineMediaThumbnail, TimelineMessageActions,
};
use crate::executor;
#[cfg(any(test, feature = "test-hooks"))]
use crate::ids::AccountKey;
use crate::ids::{RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use crate::link_preview::LinkPreviewContext;
use crate::live_tail_freshness::LiveTailRefreshCoordinator;
use crate::threads_list::ThreadRootProjectionService;

use super::actor::{TimelineActorHandle, TimelineActorMessage};
use super::manager::TimelineManagerActor;
use super::navigation::TimelineActorGenerationGate;
use super::outbound_send::{
    SendEnqueueWorkerSupervisor, SharedSendCompletionCoordinator, SubmissionAdmissionLedger,
    TimelineSendTerminalIngress,
};
use super::read_state::ReadWorkerSupervisor;
use super::thread_projection::{
    ReplayKnownThreadRootProjectionRegistry, ThreadRootProjectionFetchRegistry,
};

pub(super) fn fake_rid(seq: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(999),
        sequence: seq,
    }
}

pub(super) fn room_key() -> TimelineKey {
    TimelineKey::room(AccountKey("@a:test".to_owned()), "!r:test")
}

pub(super) async fn replacement_generation_fixture(
    key: &TimelineKey,
) -> (Arc<TimelineActorGenerationGate>, u64, u64) {
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let stale = generations.activate_after_quiescence(key).await.generation;
    let current = generations.activate_after_quiescence(key).await.generation;
    (generations, stale, current)
}

pub(super) fn replay_projection_services() -> (
    Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    Arc<Mutex<ThreadRootProjectionService>>,
) {
    (
        Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        )),
        Arc::new(Mutex::new(ThreadRootProjectionService::default())),
    )
}

pub(super) fn timeline_item(
    event_id: &str,
    body: Option<&str>,
    sender: &str,
    is_hidden: bool,
) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some(sender.to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: body.map(ToOwned::to_owned),
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
        can_react: false,
        is_redacted: false,
        is_hidden,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
    }
}

pub(super) fn test_timeline_actor_handle() -> TimelineActorHandle {
    let (tx, mut rx) = mpsc::channel(1);
    let task = executor::spawn(async move { while rx.recv().await.is_some() {} });
    TimelineActorHandle {
        tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: Some(task),
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    }
}

pub(super) fn gap_demand_test_actor_handle(
    label: &'static str,
    log: Arc<Mutex<Vec<String>>>,
) -> TimelineActorHandle {
    let (tx, mut rx) = mpsc::channel(8);
    let task = executor::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                TimelineActorMessage::BeginGapRepairDemand => log
                    .lock()
                    .expect("gap demand log lock")
                    .push(format!("begin:{label}")),
                TimelineActorMessage::EndGapRepairDemand => log
                    .lock()
                    .expect("gap demand log lock")
                    .push(format!("end:{label}")),
                TimelineActorMessage::CancelLiveTailNetwork { acknowledged, .. } => {
                    let _ = acknowledged.send(());
                }
                _ => {}
            }
        }
    });
    TimelineActorHandle {
        tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: Some(task),
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    }
}

pub(super) fn live_tail_test_manager(
    timelines: HashMap<TimelineKey, TimelineActorHandle>,
) -> TimelineManagerActor {
    let (action_tx, _action_rx) = mpsc::channel(8);
    let (event_tx, _event_rx) = broadcast::channel(8);
    let (msg_tx, msg_rx) = mpsc::channel(8);
    let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
    TimelineManagerActor {
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
        timelines,
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
        thread_root_projection_service: Arc::new(
            Mutex::new(ThreadRootProjectionService::default()),
        ),
        thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
        replay_known_thread_root_projections: Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        )),
        timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
        live_tail_refreshes: LiveTailRefreshCoordinator::new(),
        test_session_available: true,
    }
}

pub(super) fn timeline_media_item(
    event_id: &str,
    sender: &str,
    sender_label: Option<&str>,
    timestamp_ms: u64,
    filename: &str,
    kind: TimelineMediaKind,
) -> TimelineItem {
    let mut item = timeline_item(event_id, None, sender, false);
    item.sender_label = sender_label.map(ToOwned::to_owned);
    item.timestamp_ms = Some(timestamp_ms);
    item.media = Some(TimelineMedia {
        kind,
        filename: filename.to_owned(),
        source: TimelineMediaSource {
            mxc_uri: format!("mxc://example.invalid/{event_id}"),
            encrypted: true,
            encryption_version: Some("v2".to_owned()),
        },
        mimetype: Some("image/png".to_owned()),
        size: Some(2048),
        width: Some(640),
        height: Some(480),
        thumbnail: Some(TimelineMediaThumbnail {
            source: TimelineMediaSource {
                mxc_uri: format!("mxc://example.invalid/{event_id}-thumb"),
                encrypted: false,
                encryption_version: None,
            },
            mimetype: Some("image/png".to_owned()),
            size: Some(512),
            width: Some(160),
            height: Some(120),
        }),
    });
    item
}

pub(super) fn focused_key() -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@a:test".to_owned()),
        kind: TimelineKind::Focused {
            room_id: "!r:test".to_owned(),
            event_id: "$evt:test".to_owned(),
        },
    }
}

pub(super) fn thread_key() -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@a:test".to_owned()),
        kind: TimelineKind::Thread {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
        },
    }
}
