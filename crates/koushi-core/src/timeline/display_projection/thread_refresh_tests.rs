use super::root_display_item;
use crate::threads_list::{
    AggregateRefreshCause, AuthoritativeThreadAggregate, ThreadRootProjectionActivity,
    ThreadRootProjectionRefreshResult, ThreadRootProjectionService,
};
use crate::timeline::test_support::timeline_item;
use koushi_protocol::event::{ThreadSummaryDto, TimelineDisplayKind};
use koushi_state::OperationFailureKind;

fn activity() -> ThreadRootProjectionActivity {
    ThreadRootProjectionActivity {
        room_id: "!room:example.invalid".into(),
        root_event_id: "$root:example.invalid".into(),
        activity_event_id: "$reply:example.invalid".into(),
        activity_timestamp_ms: Some(100),
        activity_sender: None,
        activity_sender_label: None,
        activity_body_preview: None,
    }
}

#[test]
fn thread_refresh_keeps_loaded_root_visible_while_pending_and_after_failure() {
    for canonical in [false, true] {
        let activity = activity();
        let mut service = ThreadRootProjectionService::default();
        let mut item = timeline_item(
            &activity.root_event_id,
            Some("Synthetic root"),
            "@member:example.invalid",
            false,
        );
        item.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: Some(100),
        });
        if canonical {
            service.seed_canonical_root(&activity.room_id, &item);
        } else {
            service.observe(activity.clone());
            service.mark_ready(&activity, item.clone()).unwrap();
        }
        let refresh = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::SelectedActivity,
                true,
                false,
            )
            .unwrap();
        for failed in [false, true] {
            if failed {
                service.complete_refresh(&refresh, Err(OperationFailureKind::Sdk));
            }
            let root = service
                .display_data_for_room(&activity.room_id)
                .pop()
                .unwrap();
            let rendered =
                root_display_item(&root, &item, activity.activity_event_id.clone(), Some(100));
            assert_eq!(rendered.body, item.body);
            assert_eq!(
                rendered.display_metadata.unwrap().kind,
                TimelineDisplayKind::ThreadRoot
            );
        }
        let refresh = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::SelectedActivity,
                true,
                false,
            )
            .unwrap();
        service.complete_refresh(
            &refresh,
            Ok(ThreadRootProjectionRefreshResult::Aggregate(
                AuthoritativeThreadAggregate {
                    reply_count: 2,
                    latest_event_id: Some("$new-reply:example.invalid".into()),
                    latest_timestamp_ms: Some(200),
                    ..Default::default()
                },
            )),
        );
        let root = service
            .display_data_for_room(&activity.room_id)
            .pop()
            .unwrap();
        let rendered = root_display_item(&root, &item, root.activity_event_id.clone(), Some(200));
        assert_eq!(rendered.body, item.body);
        assert_eq!(rendered.thread_summary.unwrap().reply_count, 2);
        assert_eq!(
            rendered.display_metadata.unwrap().kind,
            TimelineDisplayKind::ThreadRoot
        );
    }
}

#[test]
fn thread_refresh_without_loaded_root_still_reports_loading_and_failure() {
    let activity = activity();
    let mut service = ThreadRootProjectionService::default();
    service.observe(activity.clone());
    let root = service
        .display_data_for_room(&activity.room_id)
        .pop()
        .unwrap();
    assert!(root.pending);
    assert!(root.item.is_none());
    service
        .mark_failed(&activity, OperationFailureKind::Sdk)
        .unwrap();
    let root = service
        .display_data_for_room(&activity.room_id)
        .pop()
        .unwrap();
    assert!(!root.pending);
    assert_eq!(root.failure_kind, Some(OperationFailureKind::Sdk));
}
