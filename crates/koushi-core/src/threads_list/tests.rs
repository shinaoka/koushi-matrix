use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, event_id, user_id};
use matrix_sdk_ui::timeline::thread_list_service::{ThreadListItemEvent, ThreadRelationAggregate};
use matrix_sdk_ui::timeline::{Profile, TimelineDetails};
use tokio::sync::{mpsc, oneshot};

use koushi_protocol::event::{
    ThreadSummaryDto, TimelineItem, TimelineItemId, TimelineMessageActions,
};

use super::{
    ActiveSubscription, AggregateRefreshCause, AuthoritativeThreadAggregate, OperationFailureKind,
    SubscriptionTasks, THREAD_SUMMARY_PROJECTION_MAX_ROOTS, ThreadRootProjectionActivity,
    ThreadRootProjectionCompletion, ThreadRootProjectionDecision,
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
        let _ = crate::executor::timeout(Duration::from_millis(100), settled)
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
        mentioned_user_ids: Vec::new(),
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
        display_metadata: None,
    }
}

fn canonical_timeline_item(event_id: &str, summary: ThreadSummaryDto) -> TimelineItem {
    let mut item = test_timeline_item(event_id);
    item.thread_summary = Some(summary);
    item
}

#[test]
fn thread_summary_projection_service_is_bounded_to_120_roots_per_room() {
    let mut service = ThreadRootProjectionService::default();
    for index in 0..THREAD_SUMMARY_PROJECTION_MAX_ROOTS {
        let activity = ThreadRootProjectionActivity {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: format!("$root-{index}:example.invalid"),
            activity_event_id: format!("$reply-{index}:example.invalid"),
            activity_timestamp_ms: Some(index as u64),
            activity_sender: None,
            activity_sender_label: None,
            activity_body_preview: None,
        };
        assert!(matches!(
            service.observe(activity),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
    }
    let extra = ThreadRootProjectionActivity {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$root-extra:example.invalid".to_owned(),
        activity_event_id: "$reply-extra:example.invalid".to_owned(),
        activity_timestamp_ms: Some(121),
        activity_sender: None,
        activity_sender_label: None,
        activity_body_preview: None,
    };
    assert_eq!(
        service.observe(extra),
        ThreadRootProjectionDecision::Retired
    );
    assert_eq!(
        service.active_activities("!room:example.invalid").len(),
        THREAD_SUMMARY_PROJECTION_MAX_ROOTS
    );
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
fn active_failed_root_survives_recreated_actor_and_empty_window_reconciliation() {
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
        service.reconcile_room_visibility(
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

    // A bounded empty window is only dormant visibility. The retained
    // terminal remains available to a replacement actor.
    {
        let mut service = shared.lock().expect("test service lock");
        service.reconcile_room_visibility(&activity.room_id, &HashSet::new());
        assert!(matches!(
            service.observe(activity),
            ThreadRootProjectionDecision::Existing(record)
                if record.failure_kind() == Some(OperationFailureKind::NotFound)
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
    service.reconcile_room_visibility(
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
fn canonical_sdk_summary_is_provisional_until_live_observation_or_refresh() {
    let mut service = ThreadRootProjectionService::default();
    service.seed_canonical_root(
        "!room:example.invalid",
        &canonical_timeline_item(
            "$root:example.invalid",
            ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some("$reply-a:example.invalid".to_owned()),
                latest_sender: Some("@a:example.invalid".to_owned()),
                latest_sender_label: Some("A".to_owned()),
                latest_body_preview: Some("A".to_owned()),
                latest_timestamp_ms: Some(100),
            },
        ),
    );
    let activity_b = ThreadRootProjectionActivity {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$root:example.invalid".to_owned(),
        activity_event_id: "$reply-b:example.invalid".to_owned(),
        activity_timestamp_ms: Some(200),
        activity_sender: Some("@b:example.invalid".to_owned()),
        activity_sender_label: Some("B".to_owned()),
        activity_body_preview: Some("B".to_owned()),
    };
    service.seed_canonical_root(
        "!room:example.invalid",
        &canonical_timeline_item(
            "$root:example.invalid",
            ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some(activity_b.activity_event_id.clone()),
                latest_sender: activity_b.activity_sender.clone(),
                latest_sender_label: activity_b.activity_sender_label.clone(),
                latest_body_preview: activity_b.activity_body_preview.clone(),
                latest_timestamp_ms: activity_b.activity_timestamp_ms,
            },
        ),
    );
    let provisional = service
        .current_aggregate("!room:example.invalid", "$root:example.invalid")
        .expect("retained accepted aggregate");
    assert_eq!(provisional.reply_count, 1);
    assert_eq!(
        provisional.latest_event_id.as_deref(),
        Some("$reply-a:example.invalid")
    );
    assert!(matches!(
        service.observe_live_activity(activity_b.clone()),
        ThreadRootProjectionDecision::ActivityUpdated(_)
    ));
    let live = service
        .current_aggregate("!room:example.invalid", "$root:example.invalid")
        .expect("live floor");
    assert_eq!(live.reply_count, 2);
    assert_eq!(
        live.latest_event_id.as_deref(),
        Some("$reply-b:example.invalid")
    );
    let refresh = service
        .schedule_aggregate_refresh_with_canonical_root(
            &activity_b,
            AggregateRefreshCause::SelectedActivity,
            true,
            true,
            false,
        )
        .expect("live aggregate refresh");
    let completion = service.complete_refresh(
        &refresh,
        Ok(ThreadRootProjectionRefreshResult::Aggregate(
            AuthoritativeThreadAggregate {
                reply_count: 1,
                latest_event_id: Some("$reply-a:example.invalid".to_owned()),
                latest_sender: Some("@a:example.invalid".to_owned()),
                latest_sender_label: Some("A".to_owned()),
                latest_body_preview: Some("A".to_owned()),
                latest_timestamp_ms: Some(100),
            },
        )),
    );
    assert!(matches!(
        completion,
        ThreadRootProjectionCompletion::Updated(record)
            if record.aggregate.reply_count == 2
                && record.aggregate.latest_event_id.as_deref()
                    == Some("$reply-b:example.invalid")
    ));
}

#[test]
fn live_observation_does_not_recount_an_activity_already_in_exact_aggregate() {
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
    let activity_b = ThreadRootProjectionActivity {
        activity_event_id: "$reply-b:example.invalid".to_owned(),
        activity_timestamp_ms: Some(200),
        activity_sender: Some("@b:example.invalid".to_owned()),
        activity_sender_label: Some("B".to_owned()),
        activity_body_preview: Some("B".to_owned()),
        ..activity_a
    };
    assert!(matches!(
        service.complete_refresh(
            &refresh_a,
            Ok(ThreadRootProjectionRefreshResult::Aggregate(
                AuthoritativeThreadAggregate {
                    reply_count: 2,
                    latest_event_id: Some(activity_b.activity_event_id.clone()),
                    latest_sender: activity_b.activity_sender.clone(),
                    latest_sender_label: activity_b.activity_sender_label.clone(),
                    latest_body_preview: activity_b.activity_body_preview.clone(),
                    latest_timestamp_ms: activity_b.activity_timestamp_ms,
                },
            )),
        ),
        ThreadRootProjectionCompletion::Updated(_)
    ));
    assert!(matches!(
        service.observe_live_activity(activity_b.clone()),
        ThreadRootProjectionDecision::ActivityUpdated(_)
    ));
    let aggregate = service
        .current_aggregate("!room:example.invalid", "$root:example.invalid")
        .expect("accepted aggregate");
    assert_eq!(aggregate.reply_count, 2);
    assert_eq!(
        aggregate.latest_event_id.as_deref(),
        Some("$reply-b:example.invalid")
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
fn older_bundled_summary_rolls_back_only_after_event_cache_confirmation() {
    let mut service = ThreadRootProjectionService::default();
    let summary_a = ThreadSummaryDto {
        reply_count: 1,
        latest_event_id: Some("$reply-a:example.invalid".to_owned()),
        latest_sender: Some("@a:example.invalid".to_owned()),
        latest_sender_label: Some("A".to_owned()),
        latest_body_preview: Some("A".to_owned()),
        latest_timestamp_ms: Some(100),
    };
    service.seed_canonical_root(
        "!room:example.invalid",
        &canonical_timeline_item("$root:example.invalid", summary_a.clone()),
    );
    let activity_b = ThreadRootProjectionActivity {
        room_id: "!room:example.invalid".to_owned(),
        root_event_id: "$root:example.invalid".to_owned(),
        activity_event_id: "$reply-b:example.invalid".to_owned(),
        activity_timestamp_ms: Some(200),
        activity_sender: Some("@b:example.invalid".to_owned()),
        activity_sender_label: Some("B".to_owned()),
        activity_body_preview: Some("B".to_owned()),
    };
    service.seed_canonical_root(
        "!room:example.invalid",
        &canonical_timeline_item(
            "$root:example.invalid",
            ThreadSummaryDto {
                reply_count: 1,
                latest_event_id: Some(activity_b.activity_event_id.clone()),
                latest_sender: activity_b.activity_sender.clone(),
                latest_sender_label: activity_b.activity_sender_label.clone(),
                latest_body_preview: activity_b.activity_body_preview.clone(),
                latest_timestamp_ms: activity_b.activity_timestamp_ms,
            },
        ),
    );
    assert!(matches!(
        service.observe_live_activity(activity_b.clone()),
        ThreadRootProjectionDecision::ActivityUpdated(_)
    ));
    assert_eq!(
        service
            .current_aggregate("!room:example.invalid", "$root:example.invalid")
            .expect("live summary")
            .reply_count,
        2
    );

    // The older bundled root alone is not enough to regress B.
    service.seed_canonical_root(
        "!room:example.invalid",
        &canonical_timeline_item("$root:example.invalid", summary_a.clone()),
    );
    assert_eq!(
        service
            .current_aggregate("!room:example.invalid", "$root:example.invalid")
            .expect("retained live summary")
            .latest_event_id
            .as_deref(),
        Some("$reply-b:example.invalid")
    );
    let refresh = service
        .schedule_aggregate_refresh_with_canonical_root(
            &activity_b,
            AggregateRefreshCause::CanonicalBatch,
            true,
            true,
            false,
        )
        .expect("rollback confirmation refresh");
    let completion = service.complete_refresh(
        &refresh,
        Ok(ThreadRootProjectionRefreshResult::Aggregate(
            AuthoritativeThreadAggregate {
                reply_count: summary_a.reply_count,
                latest_event_id: summary_a.latest_event_id,
                latest_sender: summary_a.latest_sender,
                latest_sender_label: summary_a.latest_sender_label,
                latest_body_preview: summary_a.latest_body_preview,
                latest_timestamp_ms: summary_a.latest_timestamp_ms,
            },
        )),
    );
    assert!(matches!(
        completion,
        ThreadRootProjectionCompletion::Updated(record)
            if record.aggregate.reply_count == 1
                && record.aggregate.latest_event_id.as_deref()
                    == Some("$reply-a:example.invalid")
    ));
}

#[test]
fn redaction_retires_live_floor_and_restores_exact_sdk_aggregate() {
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
    let initial = service
        .schedule_aggregate_refresh(
            &activity_a,
            AggregateRefreshCause::InitialHydration,
            true,
            false,
        )
        .expect("initial aggregate refresh");
    service.complete_refresh(
        &initial,
        Ok(ThreadRootProjectionRefreshResult::Aggregate(
            AuthoritativeThreadAggregate {
                reply_count: 1,
                latest_event_id: Some(activity_a.activity_event_id.clone()),
                latest_sender: activity_a.activity_sender.clone(),
                latest_sender_label: activity_a.activity_sender_label.clone(),
                latest_body_preview: activity_a.activity_body_preview.clone(),
                latest_timestamp_ms: activity_a.activity_timestamp_ms,
            },
        )),
    );

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
    service.complete_refresh(
        &refresh_b,
        Ok(ThreadRootProjectionRefreshResult::Aggregate(
            AuthoritativeThreadAggregate {
                reply_count: 2,
                latest_event_id: Some(activity_b.activity_event_id.clone()),
                latest_sender: activity_b.activity_sender.clone(),
                latest_sender_label: activity_b.activity_sender_label.clone(),
                latest_body_preview: activity_b.activity_body_preview.clone(),
                latest_timestamp_ms: activity_b.activity_timestamp_ms,
            },
        )),
    );

    let older_edit = ThreadRootProjectionActivity {
        activity_body_preview: Some("A edited".to_owned()),
        ..activity_a.clone()
    };
    assert!(matches!(
        service.observe_live_activity(older_edit.clone()),
        ThreadRootProjectionDecision::Existing(_)
    ));

    assert!(service.invalidate_live_activity(
        &activity_b.room_id,
        &activity_b.root_event_id,
        &activity_b.activity_event_id,
    ));
    let removal = service
        .schedule_aggregate_refresh(&activity_b, AggregateRefreshCause::Removal, true, false)
        .expect("redaction aggregate refresh");
    assert!(matches!(
        service.complete_refresh(
            &removal,
            Ok(ThreadRootProjectionRefreshResult::Aggregate(
                AuthoritativeThreadAggregate {
                    reply_count: 1,
                    latest_event_id: Some(activity_a.activity_event_id.clone()),
                    latest_sender: activity_a.activity_sender.clone(),
                    latest_sender_label: activity_a.activity_sender_label.clone(),
                    latest_body_preview: activity_a.activity_body_preview.clone(),
                    latest_timestamp_ms: activity_a.activity_timestamp_ms,
                },
            )),
        ),
        ThreadRootProjectionCompletion::Updated(_)
    ));
    assert_eq!(
        service
            .current_aggregate(&activity_a.room_id, &activity_a.root_event_id)
            .expect("restored aggregate")
            .latest_event_id
            .as_deref(),
        Some("$reply-a:example.invalid")
    );
    assert!(matches!(
        service.observe_live_activity(older_edit),
        ThreadRootProjectionDecision::ActivityUpdated(_)
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
    assert!(service.invalidate_live_activity(
        &activity_b.room_id,
        &activity_b.root_event_id,
        &activity_b.activity_event_id,
    ));
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

    service.reconcile_room_visibility(&activity_a.room_id, &HashSet::new());
    assert!(service.invalidate_live_activity(
        &activity_a.room_id,
        &activity_a.root_event_id,
        &activity_a.activity_event_id,
    ));
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
        super::ThreadRootProjectionCompletion::Cleared(cleared)
            if cleared.root_event_id == activity_a.root_event_id
    ));
    assert!(
        service
            .terminal_record(&activity_a.room_id, &activity_a.root_event_id)
            .is_none(),
        "authoritative aggregate zero explicitly clears the retained root"
    );
}

#[test]
fn hydrated_zero_count_for_inactive_root_retains_the_root_snapshot() {
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
        super::ThreadRootProjectionCompletion::Updated(record)
            if record.activity == activity && record.item().is_some()
    ));
    assert!(
        service
            .terminal_record(&activity.room_id, &activity.root_event_id)
            .is_some(),
        "a hydrated dormant root remains retained"
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
            activity_timestamp_ms: Some(200),
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
    service.reconcile_room_visibility(&activity.room_id, &HashSet::new());
    let disappeared = service
        .schedule_aggregate_refresh(&activity, AggregateRefreshCause::Removal, false, false)
        .expect("disappearance refresh");
    assert!(matches!(
        service.complete_refresh(&disappeared, Err(OperationFailureKind::Sdk)),
        super::ThreadRootProjectionCompletion::Updated(record)
            if record.failure_kind() == Some(OperationFailureKind::Sdk)
    ));
}

#[test]
fn reconciliation_moves_ready_and_failed_records_to_the_remaining_older_reply_without_fetching() {
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

        // An explicit redaction/removal authorizes the representative to
        // move backward; ordinary bounded-window disappearance does not.
        assert!(service.invalidate_live_activity(
            &newer.room_id,
            &newer.root_event_id,
            &newer.activity_event_id,
        ));
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
fn dormant_terminal_completion_retains_core_record_until_room_clear() {
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
    service.reconcile_room_visibility(&activity.room_id, &HashSet::new());

    let completed = service
        .mark_failed(&activity, OperationFailureKind::NotFound)
        .expect("the terminal result must remain available after activity leaves");
    assert_eq!(
        completed.failure_kind(),
        Some(OperationFailureKind::NotFound)
    );
    assert!(matches!(
        service.observe(activity.clone()),
        ThreadRootProjectionDecision::Existing(record)
            if record.failure_kind() == Some(OperationFailureKind::NotFound)
    ));
    assert_eq!(service.clear_room(&activity.room_id).len(), 1);
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
    service.reconcile_room_visibility(
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
        mentioned_user_ids: Vec::new(),
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
        display_metadata: None,
    };
    service
        .mark_ready(&activity, item)
        .expect("the reply remains active even while its root is canonical");

    assert!(matches!(
        service.observe(activity),
        ThreadRootProjectionDecision::Existing(record) if record.item().is_some()
    ));
}
