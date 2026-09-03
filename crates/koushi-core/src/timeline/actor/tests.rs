use std::sync::{Arc, atomic::Ordering};

use std::time::Duration;

use futures_util::StreamExt;

use tokio::sync::mpsc;

use crate::executor;

use std::sync::atomic::AtomicBool;

use super::super::outbound_send::{PendingSendPhase, PendingSendProjection, pending_send_item};
use super::super::test_support::{fake_rid, room_key, timeline_item};
use super::{
    ThreadSummaryProjectionIngress, ThreadSummaryProjectionWake, TimelineActorControl,
    TimelineActorHandle, TimelineActorMessage, canonical_pending_event_ids, should_fetch_members,
};

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[test]
fn pending_terminal_already_in_canonical_window_is_selected_for_convergence() {
    let key = room_key();
    let mut projection_item = pending_send_item("sdk-prior", "fallback", None, None, None);
    projection_item.id = koushi_protocol::event::TimelineItemId::Event {
        event_id: "$prior:test".to_owned(),
    };
    let projection = PendingSendProjection {
        key,
        sequence: 1,
        client_txn_id: "client-prior".to_owned(),
        item: projection_item,
        sdk_transaction_id: Some("sdk-prior".to_owned()),
        handle: None,
        terminal_event_id: Some("$prior:test".to_owned()),
        phase: PendingSendPhase::SentAwaitingRemote,
    };
    let canonical = vec![timeline_item(
        "$prior:test",
        Some("authoritative"),
        "@sender:test",
        false,
    )];

    assert_eq!(
        canonical_pending_event_ids(&[projection], &canonical),
        vec!["$prior:test".to_owned()]
    );
}

#[test]
fn room_and_focused_timelines_fetch_members_but_threads_reuse_room_state() {
    assert!(should_fetch_members(
        &koushi_protocol::ids::TimelineKind::Room {
            room_id: "!room:example.org".to_owned(),
        }
    ));
    assert!(should_fetch_members(
        &koushi_protocol::ids::TimelineKind::Focused {
            room_id: "!room:example.org".to_owned(),
            event_id: "$event:example.org".to_owned(),
        }
    ));
    assert!(!should_fetch_members(
        &koushi_protocol::ids::TimelineKind::Thread {
            room_id: "!room:example.org".to_owned(),
            root_event_id: "$root:example.org".to_owned(),
        }
    ));
}

#[test]
fn thread_summary_projection_watch_is_bounded_and_latest_wins() {
    let (ingress, mut receiver) = ThreadSummaryProjectionIngress::channel();
    for index in 0..crate::threads_list::THREAD_SUMMARY_PROJECTION_MAX_ROOTS {
        ingress.publish(ThreadSummaryProjectionWake::Updated {
            root_event_id: format!("$root-{index}"),
            activity_revision: 1,
            summary_revision: 1,
        });
    }
    assert_eq!(
        receiver.borrow().len(),
        crate::threads_list::THREAD_SUMMARY_PROJECTION_MAX_ROOTS
    );

    ingress.publish(ThreadSummaryProjectionWake::Updated {
        root_event_id: "$root-0".to_owned(),
        activity_revision: 2,
        summary_revision: 3,
    });
    assert_eq!(
        receiver.borrow().len(),
        crate::threads_list::THREAD_SUMMARY_PROJECTION_MAX_ROOTS
    );
    let summary_revision = match receiver.borrow().get("$root-0") {
        Some(ThreadSummaryProjectionWake::Updated {
            summary_revision, ..
        }) => *summary_revision,
        _ => panic!("updated wake expected"),
    };
    assert_eq!(summary_revision, 3);

    let drained = ingress.drain(&mut receiver);
    assert_eq!(
        drained.len(),
        crate::threads_list::THREAD_SUMMARY_PROJECTION_MAX_ROOTS
    );
    assert!(receiver.borrow().is_empty());
    assert!(
        !receiver
            .has_changed()
            .expect("projection sender remains live"),
        "draining must acknowledge the clear instead of spinning the biased select"
    );

    // A publication racing the actor's next select belongs to the new
    // watch value rather than the atomically drained batch.
    ingress.publish(ThreadSummaryProjectionWake::Updated {
        root_event_id: "$root-new".to_owned(),
        activity_revision: 4,
        summary_revision: 5,
    });
    assert_eq!(receiver.borrow().len(), 1);
    let drained = ingress.drain(&mut receiver);
    assert!(matches!(
        drained.as_slice(),
        [ThreadSummaryProjectionWake::Updated { root_event_id, .. }]
            if root_event_id == "$root-new"
    ));
}

#[test]
fn thread_summary_projection_clear_ordering_is_latest_wins() {
    let (ingress, mut receiver) = ThreadSummaryProjectionIngress::channel();
    // A newer Updated for the same root supersedes an older Cleared.
    ingress.publish(ThreadSummaryProjectionWake::Cleared {
        root_event_id: "$root:test".to_owned(),
        activity_revision: 1,
        summary_revision: 1,
    });
    ingress.publish(ThreadSummaryProjectionWake::Updated {
        root_event_id: "$root:test".to_owned(),
        activity_revision: 2,
        summary_revision: 1,
    });
    let wake = receiver
        .borrow()
        .get("$root:test")
        .cloned()
        .expect("wake present");
    assert!(matches!(
        wake,
        ThreadSummaryProjectionWake::Updated {
            activity_revision: 2,
            ..
        }
    ));

    // An older Updated cannot un-clear a newer Cleared.
    ingress.publish(ThreadSummaryProjectionWake::Cleared {
        root_event_id: "$root:test".to_owned(),
        activity_revision: 3,
        summary_revision: 2,
    });
    ingress.publish(ThreadSummaryProjectionWake::Updated {
        root_event_id: "$root:test".to_owned(),
        activity_revision: 1,
        summary_revision: 1,
    });
    let wake = receiver
        .borrow()
        .get("$root:test")
        .cloned()
        .expect("wake present");
    assert!(matches!(
        wake,
        ThreadSummaryProjectionWake::Cleared {
            activity_revision: 3,
            ..
        }
    ));

    // A later equal-revision publication is the latest service truth: a
    // recreated Updated may supersede a prior Clear without growing the map.
    ingress.publish(ThreadSummaryProjectionWake::Cleared {
        root_event_id: "$root:test".to_owned(),
        activity_revision: 4,
        summary_revision: 4,
    });
    ingress.publish(ThreadSummaryProjectionWake::Updated {
        root_event_id: "$root:test".to_owned(),
        activity_revision: 4,
        summary_revision: 4,
    });
    let drained = ingress.drain(&mut receiver);
    assert_eq!(drained.len(), 1);
    assert!(matches!(
        drained.as_slice(),
        [ThreadSummaryProjectionWake::Updated {
            activity_revision: 4,
            summary_revision: 4,
            ..
        }]
    ));
    assert!(receiver.borrow().is_empty());
}

#[test]
fn replaced_thread_summary_projection_watch_drops_old_values() {
    let (ingress, receiver) = ThreadSummaryProjectionIngress::channel();
    ingress.publish(ThreadSummaryProjectionWake::Updated {
        root_event_id: "$old-root".to_owned(),
        activity_revision: 1,
        summary_revision: 1,
    });
    drop(ingress);
    assert!(receiver.has_changed().is_err());
}

#[tokio::test]
async fn timeline_actor_handle_drop_aborts_actor_and_auxiliary_tasks() {
    let actor_alive = Arc::new(AtomicBool::new(true));
    let auxiliary_alive = Arc::new(AtomicBool::new(true));
    let (tx, mut rx) = mpsc::channel(1);
    let (actor_started_tx, actor_started_rx) = tokio::sync::oneshot::channel();
    let (auxiliary_started_tx, auxiliary_started_rx) = tokio::sync::oneshot::channel();
    let actor_alive_for_task = actor_alive.clone();
    let actor_task = executor::spawn(async move {
        let _guard = DropFlag(actor_alive_for_task);
        let _ = actor_started_tx.send(());
        while rx.recv().await.is_some() {}
    });
    let auxiliary_alive_for_task = auxiliary_alive.clone();
    let auxiliary_task = executor::spawn(async move {
        let _guard = DropFlag(auxiliary_alive_for_task);
        let _ = auxiliary_started_tx.send(());
        futures_util::future::pending::<()>().await;
    });
    let auxiliary_sender = tx.clone();

    actor_started_rx.await.expect("actor task should start");
    auxiliary_started_rx
        .await
        .expect("auxiliary task should start");

    let handle = TimelineActorHandle {
        tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: Some(actor_task),
        auxiliary_tasks: vec![auxiliary_task],
        subscription_generation: None,
        enqueue_context: None,
    };
    drop(handle);
    executor::sleep(Duration::from_millis(25)).await;

    assert!(!actor_alive.load(Ordering::SeqCst));
    assert!(!auxiliary_alive.load(Ordering::SeqCst));
    assert!(
        auxiliary_sender
            .try_send(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(fake_rid(99)),
            })
            .is_err()
    );
}

#[tokio::test]
async fn timeline_actor_control_lane_bypasses_full_ordinary_mailbox() {
    let (tx, mut ordinary_rx) = mpsc::channel(1);
    tx.try_send(TimelineActorMessage::OwnReadReceiptChanged)
        .expect("ordinary mailbox prefill");
    let (control_tx, mut control_rx) = mpsc::channel(1);
    let handle = TimelineActorHandle {
        tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };

    assert!(
        executor::timeout(
            Duration::from_millis(100),
            handle.send_control(TimelineActorControl::BeginGapRepairDemand),
        )
        .await
        .expect("foreground control admission must be bounded")
    );
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::BeginGapRepairDemand)
    ));
    assert!(matches!(
        ordinary_rx.recv().await,
        Some(TimelineActorMessage::OwnReadReceiptChanged)
    ));
}
