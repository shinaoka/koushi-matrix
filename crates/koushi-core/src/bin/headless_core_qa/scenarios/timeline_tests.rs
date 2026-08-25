use super::{
    QaVisibleGapCapture, ReconnectProjection, RoomThreadSummaryObserver,
    assert_room_timeline_exposes_canonical_reply_and_summarizes_root, assert_thread_reply_relation,
    observe_reconnect_pagination_state, select_visible_gap_for_qa,
    thread_initial_items_need_paginate_backfill, thread_reply_should_repaginate_on_idle,
    timeline_item_has_thread_summary_reply, timeline_item_has_visible_payload,
};
use crate::contracts::{
    reconnect_test_bodies, reconnect_test_items, reconnect_test_request, synthetic_timeline_item,
};
use crate::diagnostics::QaCannedMessagesPage;
use crate::event_wait::{find_timeline_item_with_body, projection_timeline_item};
use crate::registry::{
    QaScenario, QaStage, TIMELINE_RECONNECT_EXPECTED_BODY_COUNT, final_tokens_for_scenario,
    stages_for_scenario,
};
use crate::{
    PaginationState, TimelineDiff, TimelineGapId, TimelineGapPosition, TimelineItemId,
    TimelineMessageActions,
};
use koushi_core::event::ThreadSummaryDto;

#[test]
fn reconnect_initial_projection_rejects_missing_newest_body() {
    let bodies = reconnect_test_bodies();
    let error =
        ReconnectProjection::from_initial(&reconnect_test_items(0..20), &bodies, "reconnect test")
            .err()
            .expect("an initial projection missing newest body 20 must be rejected");

    assert!(error.contains("newest_window_count=false"));
    assert!(!error.contains("synthetic body"));
}

#[test]
fn reconnect_initial_projection_rejects_oldest_present_before_page() {
    let bodies = reconnect_test_bodies();
    let error = ReconnectProjection::from_initial(
        &reconnect_test_items(0..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT),
        &bodies,
        "reconnect test",
    )
    .err()
    .expect("an initial projection containing oldest body 0 must be rejected");

    assert!(error.contains("oldest_count=1"));
    assert!(!error.contains("synthetic body"));
}

#[test]
fn reconnect_initial_projection_requires_mandatory_pagination() {
    let error = ReconnectProjection::from_initial(
        &reconnect_test_items(0..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT),
        &reconnect_test_bodies(),
        "reconnect test",
    )
    .err()
    .expect("the all-21 shortcut fixture must be removed");

    assert!(error.contains("initial projection"));
    assert!(!error.contains("needs no pagination"));
}

#[test]
fn reconnect_pagination_requires_paginating_before_terminal() {
    let request_id = reconnect_test_request(1);
    let mut saw_paginating = false;
    let mut terminal = false;

    observe_reconnect_pagination_state(
        Some(request_id),
        request_id,
        &PaginationState::Paginating,
        &mut saw_paginating,
        &mut terminal,
        "reconnect test",
    )
    .expect("Paginating should be accepted");
    assert!(saw_paginating);
    assert!(!terminal);

    observe_reconnect_pagination_state(
        Some(request_id),
        request_id,
        &PaginationState::Idle,
        &mut saw_paginating,
        &mut terminal,
        "reconnect test",
    )
    .expect("Idle after Paginating should be accepted");
    assert!(terminal);
}

#[test]
fn reconnect_projection_applies_destructive_diffs_exactly_and_rejects_duplicates() {
    let bodies = reconnect_test_bodies();
    let mut projection = ReconnectProjection::from_initial(
        &reconnect_test_items(1..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT),
        &bodies,
        "reconnect test",
    )
    .expect("newest-window initial projection should be valid");
    projection
        .apply_batch(&[TimelineDiff::Remove { index: 0 }], "reconnect test")
        .expect("remove should be applied");
    assert_eq!(projection.missing_indices(), vec![0, 1]);

    let newest_window_first = reconnect_test_items([1]).remove(0);
    projection
        .apply_batch(
            &[TimelineDiff::PushFront {
                item: newest_window_first,
            }],
            "reconnect test",
        )
        .expect("push front should restore the exact newest-window body");
    let first = reconnect_test_items([0]).remove(0);
    projection
        .apply_batch(&[TimelineDiff::PushFront { item: first }], "reconnect test")
        .expect("push front should recover the oldest body");
    assert!(projection.is_complete());

    let duplicate = reconnect_test_items([0]).remove(0);
    let error = projection
        .apply_batch(
            &[TimelineDiff::Set {
                index: 1,
                item: duplicate,
            }],
            "reconnect test",
        )
        .expect_err("a destructive/set diff creating a duplicate must fail");
    assert!(error.contains("duplicate_indices=[0]"));

    projection
        .apply_batch(
            &[
                TimelineDiff::Clear,
                TimelineDiff::Reset {
                    items: reconnect_test_items(0..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT),
                },
            ],
            "reconnect test",
        )
        .expect("clear/reset should rebuild the exact projection");
    assert!(projection.is_complete());
}

#[test]
fn reconnect_terminal_before_paginating_is_rejected() {
    let request_id = reconnect_test_request(2);
    let mut saw_paginating = false;
    let mut terminal = false;
    let error = observe_reconnect_pagination_state(
        Some(request_id),
        request_id,
        &PaginationState::EndReached,
        &mut saw_paginating,
        &mut terminal,
        "reconnect test",
    )
    .expect_err("terminal before Paginating must fail");

    assert!(error.contains("before Paginating"));
    assert!(!terminal);
}

#[test]
fn reconnect_terminal_can_precede_the_final_diff() {
    let bodies = reconnect_test_bodies();
    let mut projection = ReconnectProjection::from_initial(
        &reconnect_test_items(1..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT),
        &bodies,
        "reconnect test",
    )
    .expect("20-body newest-window initial projection should be valid");
    let request_id = reconnect_test_request(3);
    let mut saw_paginating = false;
    let mut terminal = false;
    observe_reconnect_pagination_state(
        Some(request_id),
        request_id,
        &PaginationState::Paginating,
        &mut saw_paginating,
        &mut terminal,
        "reconnect test",
    )
    .unwrap();
    observe_reconnect_pagination_state(
        Some(request_id),
        request_id,
        &PaginationState::EndReached,
        &mut saw_paginating,
        &mut terminal,
        "reconnect test",
    )
    .unwrap();
    assert!(terminal);
    projection
        .apply_batch(
            &[TimelineDiff::PushBack {
                item: reconnect_test_items([0]).remove(0),
            }],
            "reconnect test",
        )
        .unwrap();
    assert!(projection.is_complete());
}

#[test]
fn reconnect_projection_rejects_undecipherable_items() {
    let bodies = reconnect_test_bodies();
    let mut items = reconnect_test_items(1..TIMELINE_RECONNECT_EXPECTED_BODY_COUNT);
    items[0].body = Some("Unable to decrypt message".to_owned());
    let error = ReconnectProjection::from_initial(&items, &bodies, "reconnect test")
        .err()
        .expect("UTD must fail closed");

    assert!(error.contains("contains UTD"));
    assert!(!error.contains("synthetic body"));
}

#[test]
fn visible_gap_selector_prefers_internal_gap_and_returns_nearest_event_bounds() {
    let mut synthetic = projection_timeline_item("$synthetic-placeholder:test", false);
    synthetic.id = TimelineItemId::Synthetic {
        synthetic_id: "placeholder".to_owned(),
    };
    let items = vec![
        projection_timeline_item("$far-left:test", false),
        projection_timeline_item("$near-left:test", false),
        synthetic,
        projection_timeline_item("$near-right:test", false),
        projection_timeline_item("$far-right:test", false),
    ];
    let top_row_id = TimelineGapId {
        topology_revision: 10,
        ordinal: 0,
    };
    let bracketed_id = TimelineGapId {
        topology_revision: 10,
        ordinal: 1,
    };

    let selected = select_visible_gap_for_qa(
        &items,
        &[
            TimelineGapPosition {
                id: top_row_id,
                before_item_index: 0,
            },
            TimelineGapPosition {
                id: bracketed_id,
                before_item_index: 3,
            },
        ],
    )
    .expect("an internally bracketed gap should be visible");

    assert_eq!(selected.id, bracketed_id);
    assert_eq!(
        selected.first_visible_event_id.as_deref(),
        Some("$near-left:test")
    );
    assert_eq!(
        selected.last_visible_event_id.as_deref(),
        Some("$near-right:test")
    );
}

#[test]
fn visible_gap_selector_chooses_newest_internal_gap_from_reversed_positions() {
    let items = vec![
        projection_timeline_item("$event-0:test", false),
        projection_timeline_item("$event-1:test", false),
        projection_timeline_item("$event-2:test", false),
        projection_timeline_item("$event-3:test", false),
        projection_timeline_item("$event-4:test", false),
    ];
    let older_gap_id = TimelineGapId {
        topology_revision: 20,
        ordinal: 0,
    };
    let newest_gap_id = TimelineGapId {
        topology_revision: 21,
        ordinal: 1,
    };

    let selected = select_visible_gap_for_qa(
        &items,
        &[
            TimelineGapPosition {
                id: newest_gap_id,
                before_item_index: 4,
            },
            TimelineGapPosition {
                id: older_gap_id,
                before_item_index: 2,
            },
        ],
    )
    .expect("the newest internally bracketed gap should be visible");

    assert_eq!(selected.id, newest_gap_id);
    assert_eq!(
        selected.first_visible_event_id.as_deref(),
        Some("$event-3:test")
    );
    assert_eq!(
        selected.last_visible_event_id.as_deref(),
        Some("$event-4:test")
    );
}

#[test]
fn visible_gap_selector_chooses_newest_top_row_gap_without_event_bounds() {
    let older_gap_id = TimelineGapId {
        topology_revision: 11,
        ordinal: 0,
    };
    let newest_gap_id = TimelineGapId {
        topology_revision: 12,
        ordinal: 1,
    };
    let selected = select_visible_gap_for_qa(
        &[projection_timeline_item("$first:test", false)],
        &[
            TimelineGapPosition {
                id: newest_gap_id,
                before_item_index: 0,
            },
            TimelineGapPosition {
                id: older_gap_id,
                before_item_index: 0,
            },
        ],
    )
    .expect("a top-row gap should support a gap-only viewport");

    assert_eq!(selected.id, newest_gap_id);
    assert_eq!(selected.first_visible_event_id, None);
    assert_eq!(selected.last_visible_event_id, None);
}

#[test]
fn visible_gap_selector_rejects_unbracketed_non_top_gaps_privately() {
    let mut synthetic = projection_timeline_item("$synthetic-placeholder:test", false);
    synthetic.id = TimelineItemId::Synthetic {
        synthetic_id: "placeholder".to_owned(),
    };
    let error = select_visible_gap_for_qa(
        &[projection_timeline_item("$left:test", false), synthetic],
        &[
            TimelineGapPosition {
                id: TimelineGapId {
                    topology_revision: 12,
                    ordinal: 0,
                },
                before_item_index: 1,
            },
            TimelineGapPosition {
                id: TimelineGapId {
                    topology_revision: 12,
                    ordinal: 1,
                },
                before_item_index: 3,
            },
        ],
    )
    .expect_err("offscreen non-top gaps should not be reported as visible");

    assert!(error.contains("item_count=2"));
    assert!(error.contains("position_count=2"));
    assert!(error.contains("min_before_item_index=1"));
    assert!(error.contains("max_before_item_index=3"));
    assert!(!error.contains("$left:test"));
}

#[test]
fn visible_gap_capture_requires_a_post_body_projection() {
    let expected_body = "detached live-tail body";
    let pre_body_items = vec![
        projection_timeline_item("$old-left:test", false),
        projection_timeline_item("$old-right:test", false),
    ];
    let old_gap_id = TimelineGapId {
        topology_revision: 30,
        ordinal: 0,
    };
    let new_gap_id = TimelineGapId {
        topology_revision: 31,
        ordinal: 0,
    };
    let mut capture = QaVisibleGapCapture::default();

    capture
        .observe_items(&pre_body_items, expected_body, "ordering test")
        .unwrap();
    capture
        .observe_gap_positions(
            &pre_body_items,
            7,
            40,
            &[TimelineGapPosition {
                id: old_gap_id,
                before_item_index: 1,
            }],
            "ordering test",
        )
        .unwrap();
    assert!(capture.projected_gap().is_none());

    let mut body_item = projection_timeline_item("$new-right:test", false);
    body_item.body = Some(expected_body.to_owned());
    let post_body_items = vec![projection_timeline_item("$new-left:test", false), body_item];
    capture
        .observe_items(&post_body_items, expected_body, "ordering test")
        .unwrap();
    assert!(capture.projected_gap().is_none());

    capture
        .observe_gap_positions(
            &post_body_items,
            7,
            41,
            &[TimelineGapPosition {
                id: new_gap_id,
                before_item_index: 1,
            }],
            "ordering test",
        )
        .unwrap();
    let (selected, (actor_generation, projection_generation)) = capture
        .projected_gap()
        .expect("the post-body projection should be captured");
    assert_eq!(selected.id, new_gap_id);
    assert_eq!(*actor_generation, 7);
    assert_eq!(*projection_generation, 41);
}

#[test]
fn finds_timeline_item_in_initial_items_by_body_substring() {
    let items = vec![
        koushi_core::event::TimelineItem {
            request_state: None,
            id: koushi_core::event::TimelineItemId::Synthetic {
                synthetic_id: "skip".to_owned(),
            },
            sender: None,
            sender_label: None,
            sender_avatar: None,
            body: Some("first item".to_owned()),
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
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        },
        koushi_core::event::TimelineItem {
            request_state: None,
            id: koushi_core::event::TimelineItemId::Event {
                event_id: "$thread:test".to_owned(),
            },
            sender: Some("@b:test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("Phase 5 QA thread reply from B".to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: Some("$root:test".to_owned()),
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
            can_edit: true,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        },
    ];

    let item = find_timeline_item_with_body(&items, "thread reply from B")
        .expect("expected to find thread reply in initial items");

    assert_eq!(item.in_reply_to_event_id, Some("$root:test".to_owned()));
    assert_eq!(item.body.as_deref(), Some("Phase 5 QA thread reply from B"));
}

#[test]
fn thread_reply_missing_from_initial_items_requires_paginate_backfill() {
    let initial_items = vec![koushi_core::event::TimelineItem {
        request_state: None,
        id: koushi_core::event::TimelineItemId::Synthetic {
            synthetic_id: "placeholder".to_owned(),
        },
        sender: None,
        sender_label: None,
        sender_avatar: None,
        body: Some("Phase 5 QA message 1".to_owned()),
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
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
    }];

    assert!(thread_initial_items_need_paginate_backfill(
        &initial_items,
        "Phase 5 QA thread reply from B"
    ));
}

#[test]
fn thread_reply_present_in_initial_items_does_not_require_backfill() {
    let initial_items = vec![koushi_core::event::TimelineItem {
        request_state: None,
        id: koushi_core::event::TimelineItemId::Synthetic {
            synthetic_id: "thread-reply".to_owned(),
        },
        sender: Some("@b:test".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("Phase 5 QA thread reply from B".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: None,
        in_reply_to_event_id: Some("$root:test".to_owned()),
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
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
    }];

    assert!(!thread_initial_items_need_paginate_backfill(
        &initial_items,
        "Phase 5 QA thread reply from B"
    ));
}

#[test]
fn thread_reply_stops_repagination_after_end_reached() {
    assert!(thread_reply_should_repaginate_on_idle(false));
    assert!(!thread_reply_should_repaginate_on_idle(true));
}

#[test]
fn thread_summary_helper_requires_root_item_with_reply_count() {
    let summary = ThreadSummaryDto {
        reply_count: 1,
        latest_event_id: None,
        latest_sender: None,
        latest_sender_label: None,
        latest_body_preview: None,
        latest_timestamp_ms: None,
    };
    let root = synthetic_timeline_item("$root:test", None, None, None, Some(summary.clone()));
    let no_replies = synthetic_timeline_item(
        "$root:test",
        None,
        None,
        None,
        Some(ThreadSummaryDto {
            reply_count: 0,
            ..summary.clone()
        }),
    );
    let other_root =
        synthetic_timeline_item("$other:test", None, None, None, Some(summary.clone()));

    assert!(timeline_item_has_thread_summary_reply(&root, "$root:test"));
    assert!(!timeline_item_has_thread_summary_reply(
        &no_replies,
        "$root:test"
    ));
    assert!(!timeline_item_has_thread_summary_reply(
        &other_root,
        "$root:test"
    ));
}

#[test]
fn room_thread_assertion_requires_canonical_reply_and_root_summary() {
    let root = synthetic_timeline_item(
        "$root:test",
        Some("root message"),
        None,
        None,
        Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: None,
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: None,
        }),
    );
    let unrelated = synthetic_timeline_item("$other:test", Some("other"), None, None, None);

    assert!(
        assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
            &[root.clone(), unrelated],
            "Phase 11 QA thread reply from B",
            "$root:test",
        )
        .is_err(),
        "a Room canonical stream must include the thread reply as the projection anchor"
    );

    let canonical_reply = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        Some("$root:test"),
        Some("$root:test"),
        None,
    );
    assert!(
        assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
            &[root.clone(), canonical_reply],
            "Phase 11 QA thread reply from B",
            "$root:test",
        )
        .is_ok()
    );

    assert!(
        assert_room_timeline_exposes_canonical_reply_and_summarizes_root(
            &[synthetic_timeline_item(
                "$root:test",
                Some("root message"),
                None,
                None,
                None,
            )],
            "Phase 11 QA thread reply from B",
            "$root:test",
        )
        .is_err()
    );
}

#[test]
fn room_thread_summary_observer_waits_for_late_summary_diff() {
    let mut observer = RoomThreadSummaryObserver::new(
        "Phase 11 QA thread reply from B",
        "$reply:test",
        1,
        "$root:test",
    );
    let root_without_summary =
        synthetic_timeline_item("$root:test", Some("root message"), None, None, None);

    assert!(!observer.observe_items(&[root_without_summary]).unwrap());

    let root_with_summary = synthetic_timeline_item(
        "$root:test",
        Some("root message"),
        None,
        None,
        Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: None,
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: None,
            latest_timestamp_ms: None,
        }),
    );

    assert!(
        observer
            .observe_diffs(&[TimelineDiff::Set {
                index: 0,
                item: root_with_summary,
            }])
            .unwrap()
            == false,
        "the root summary alone is insufficient; canonical reply observation is the anchor contract"
    );
}

#[test]
fn room_thread_summary_observer_rejects_stale_non_null_summary_until_rust_advances_it() {
    let mut observer =
        RoomThreadSummaryObserver::new("new live reply", "$reply-b:test", 2, "$root:test");
    let stale_root = synthetic_timeline_item(
        "$root:test",
        Some("root message"),
        None,
        None,
        Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$reply-a:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: Some("old reply".to_owned()),
            latest_timestamp_ms: Some(100),
        }),
    );
    let live_reply = synthetic_timeline_item(
        "$reply-b:test",
        Some("new live reply"),
        Some("$root:test"),
        Some("$root:test"),
        None,
    );
    assert!(!observer.observe_items(&[stale_root]).unwrap());
    assert!(!observer.observe_items(&[live_reply]).unwrap());

    let current_root = synthetic_timeline_item(
        "$root:test",
        Some("root message"),
        None,
        None,
        Some(ThreadSummaryDto {
            reply_count: 2,
            latest_event_id: Some("$reply-b:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: Some("new live reply".to_owned()),
            latest_timestamp_ms: Some(200),
        }),
    );
    assert!(
        observer
            .observe_diffs(&[TimelineDiff::Set {
                index: 0,
                item: current_root,
            }])
            .unwrap()
    );
}

#[test]
fn room_thread_summary_observer_accepts_canonical_thread_reply() {
    let mut observer = RoomThreadSummaryObserver::new(
        "Phase 11 QA thread reply from B",
        "$reply:test",
        1,
        "$root:test",
    );
    let canonical_reply = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        Some("$root:test"),
        Some("$root:test"),
        None,
    );

    assert!(!observer.observe_items(&[canonical_reply]).unwrap());
}

#[test]
fn thread_qa_reports_canonical_reply_contract() {
    assert!(
        final_tokens_for_scenario(QaScenario::Thread).contains(&"thread_canonical=ok"),
        "the public QA summary must describe the canonical Room stream contract"
    );
}

#[test]
fn thread_relation_helper_requires_thread_root_and_validates_optional_reply_metadata() {
    let valid = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        Some("$root:test"),
        Some("$root:test"),
        None,
    );
    let valid_thread_only = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        None,
        Some("$root:test"),
        None,
    );
    let mismatched_reply = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        Some("$other:test"),
        Some("$root:test"),
        None,
    );
    let missing_thread_root = synthetic_timeline_item(
        "$reply:test",
        Some("Phase 11 QA thread reply from B"),
        Some("$root:test"),
        None,
        None,
    );

    assert_thread_reply_relation(&valid, "$root:test").unwrap();
    assert_thread_reply_relation(&valid_thread_only, "$root:test").unwrap();
    assert!(assert_thread_reply_relation(&mismatched_reply, "$root:test").is_err());
    assert!(assert_thread_reply_relation(&missing_thread_root, "$root:test").is_err());
}

#[test]
fn send_queue_scenario_skips_generic_fixture_stages_and_reports_private_tokens() {
    assert!(QaScenario::SendQueue.should_run_stage(QaStage::Safety));
    assert!(QaScenario::SendQueue.should_run_stage(QaStage::LoginSync));
    assert!(!QaScenario::SendQueue.should_run_stage(QaStage::RoomSpace));
    assert!(!QaScenario::SendQueue.should_run_stage(QaStage::Timeline));
    assert!(QaScenario::SendQueue.should_run_stage(QaStage::SendQueue));
    assert!(!QaScenario::SendQueue.should_run_stage(QaStage::Reply));
    assert!(!QaScenario::SendQueue.should_run_stage(QaStage::EditRedactSearch));
    assert_eq!(
        stages_for_scenario(QaScenario::SendQueue),
        [QaStage::Safety, QaStage::LoginSync, QaStage::SendQueue]
    );

    assert_eq!(
        final_tokens_for_scenario(QaScenario::SendQueue),
        [
            "safety=ok",
            "login_sync=ok",
            "send_fail=ok",
            "resend=ok",
            "cancel_send=ok",
            "fifo=ok",
            "unsent_restart=ok",
            "display_projection_reset_fallbacks=0",
            "restore_cleanup=ok",
        ]
    );
}

#[test]
fn canned_live_tail_messages_page_reproduces_a_gap_before_the_known_latest_event() {
    let body = QaCannedMessagesPage::anchored_silent_gap(
        "$latest:example.invalid".to_owned(),
        "known latest".to_owned(),
        "$missing:example.invalid".to_owned(),
        "missing before latest".to_owned(),
        "$older:example.invalid".to_owned(),
        "@sender:example.invalid".to_owned(),
        "known older anchor".to_owned(),
    )
    .response_body()
    .expect("canned /messages response should serialize");
    let response: serde_json::Value =
        serde_json::from_slice(&body).expect("canned /messages response should be JSON");

    assert_eq!(
        response.get("start").and_then(serde_json::Value::as_str),
        Some("qa-live-tail-start")
    );
    assert!(response.get("end").is_none());
    let ids = response["chunk"]
        .as_array()
        .expect("canned chunk")
        .iter()
        .map(|event| event["event_id"].as_str().expect("event id"))
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "$latest:example.invalid",
            "$missing:example.invalid",
            "$older:example.invalid",
        ]
    );
}

#[test]
fn timeline_stress_blank_row_detection_rejects_empty_formatted_body() {
    let mut item = synthetic_timeline_item(
        "$formatted-blank:test",
        Some("plain fallback"),
        None,
        None,
        None,
    );
    item.formatted = Some(koushi_core::event::TimelineFormattedBody {
        html: "<p><br /></p>".to_owned(),
        plain_text: String::new(),
        code_blocks: Vec::new(),
    });
    item.body = None;

    assert!(
        !timeline_item_has_visible_payload(&item),
        "blank formatted HTML must not satisfy stress_no_blank"
    );
}

#[test]
fn scheduled_send_scenario_runs_after_timeline_and_reports_private_tokens() {
    assert_eq!(
        QaScenario::from_env_value("scheduled_send").unwrap(),
        QaScenario::ScheduledSend
    );
    assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::Safety));
    assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::LoginSync));
    assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::RoomSpace));
    assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::Timeline));
    assert!(QaScenario::ScheduledSend.should_run_stage(QaStage::ScheduledSend));
    assert!(QaScenario::ScheduledSend.suppress_matrix_identifiers());
    assert!(!QaScenario::ScheduledSend.should_run_stage(QaStage::Reply));
    assert!(!QaScenario::ScheduledSend.should_run_stage(QaStage::EditRedactSearch));

    assert_eq!(
        final_tokens_for_scenario(QaScenario::ScheduledSend),
        [
            "safety=ok",
            "login_sync=ok",
            "room_space=ok",
            "timeline=ok",
            "timeline_nav=ok",
            "hide_redacted=ok",
            "scheduled_capability=local_fallback",
            "scheduled_create=ok",
            "scheduled_reschedule=ok",
            "scheduled_cancel=ok",
            "scheduled_fire=ok",
            "restore_cleanup=ok",
        ]
    );
}
