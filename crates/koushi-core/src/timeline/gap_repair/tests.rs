use super::super::test_source::item_body;

use std::collections::{BTreeSet, HashMap};

use std::sync::Arc;

use std::time::Duration;

use koushi_sdk::{
    MatrixClientSession, MatrixLiveTailRefreshOutcome, MatrixTimelineGapError,
    MatrixTimelineGapRepairBudget, MatrixTimelineGapRepairOutcome, MatrixTimelineGapRepairResult,
};

use koushi_state::AppAction;

use tokio::sync::{broadcast, mpsc, oneshot};

use crate::account_work::AccountWorkKind;
#[cfg(test)]
use crate::causal_projection::CAUSAL_PROJECTION_SERIAL_MAX;
use crate::causal_projection::CausalProjectionId;
use koushi_protocol::command::TimelineCommand;
use koushi_protocol::event::{
    CoreEvent, TimelineDiff, TimelineEvent, TimelineGapId, TimelineGapPosition, TimelineItem,
    TimelineItemId, TimelineMessageActions,
};

#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::ids::AccountKey;
use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};

use koushi_state::SessionInfo;

use super::super::actor::TimelineActorMessage;
use super::super::diagnostics::{
    TimelineGapSelectionDiagnostic, record_timeline_gap_demand, record_timeline_gap_projection,
    record_timeline_gap_projection_boundary, record_timeline_gap_repair_evaluation,
    record_timeline_gap_selection,
};
use super::super::display_projection::apply_timeline_diffs_to_display_items;
use super::super::manager::TimelineMessage;
use super::super::relay::TimelineRelayBatch;
use super::super::test_support::{fake_rid, live_tail_test_manager};
use super::super::thread_projection::ThreadAttentionBatchProvenance;
use super::{
    GapBoundaryPresenceCounts, GapProjectionRelayed, GapRepairEvaluationDiagnosticSignature,
    GapRepairSelection, GapRepairViewportWakeDecision, GlobalCommitDecision, GlobalCommitFence,
    GlobalResponseCommit, LiveEdgeGapSelection, LiveEdgeSelectionDecision,
    MAX_LIVE_EDGE_GAP_REPAIR_BATCHES, MAX_TIMELINE_GAP_REPAIR_BATCHES, MissingCommittedGapDecision,
    PendingTimelineGapProjection, ProjectedGapCandidate, ProjectedGapRelation,
    TestGapRepairCompletionPause, TimelineGapAttemptResetReason, TimelineGapObservableSettlement,
    TimelineGapProjectionCompletion, TimelineGapProjectionCorrelation, TimelineGapRepairTracker,
    TimelineGapRepairTrigger, UnlocatedGapAction, UnprojectedGapReason,
    admit_and_record_timeline_gap_repair_attempt, evaluate_gap_repair_viewport_wake,
    gap_projection_relay_is_current, gap_repair_continuation_trigger, gap_repair_work_kind,
    gap_selection_diagnostic_decision, global_commit_gap_selection,
    historical_causal_projection_operation, is_global_commit_inspection_target,
    live_tail_completion_requires_snapshot, missing_committed_gap_decision,
    post_diff_gap_inspection_trigger, projected_gap_id, projected_gap_identity_matches_descriptor,
    projected_gap_insertion_index, record_timeline_gap_repair_result,
    recover_obsolete_gap_settlement, rendered_live_edge_target, select_gap_repair_candidate,
    select_projected_gap_candidate, select_projected_gap_id, should_record_gap_repair_evaluation,
    summarize_gap_boundary_presence, timeline_gap_repair_budget, timeline_gap_repair_made_progress,
    timeline_gap_repair_result_diagnostic, timeline_gap_repair_trigger_token, unlocated_gap_action,
    wait_for_gap_repair_projection_with_timeout,
};

fn event_item(event_id: &str, body: &str) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: None,
        sender_label: None,
        sender_avatar: None,
        body: Some(body.to_owned()),
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
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    }
}
fn projected_gap_position(
    topology_revision: u64,
    ordinal: usize,
    before_item_index: usize,
) -> TimelineGapPosition {
    TimelineGapPosition {
        id: projected_gap_id(topology_revision, ordinal),
        before_item_index,
    }
}
fn timeline_gap_repair_diagnostic_count_since(
    diagnostic_start: usize,
    stage: &str,
    demand_revision: u64,
) -> usize {
    koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
        .iter()
        .filter(|record| {
            record.event.source == "core.timeline_gap_repair"
                && record.event.stage == stage
                && record.event.fields.iter().any(|field| {
                    field.key == "demand_revision"
                        && field.value
                            == koushi_diagnostics::DiagnosticValue::Count(demand_revision)
                })
        })
        .count()
}

#[test]
fn global_commit_fence_admits_one_omitted_room_inspection_per_new_commit() {
    let mut fence = GlobalCommitFence::default();
    let covered = GlobalResponseCommit::new(7, 10);
    let omitted = GlobalResponseCommit::new(7, 11);

    fence.note_room_checkpoint_advanced(10);
    assert_eq!(
        fence.observe(covered),
        GlobalCommitDecision::CoveredByRoomCheckpoint
    );
    assert_eq!(fence.take_pending_inspection(), None);

    assert_eq!(
        fence.observe(omitted),
        GlobalCommitDecision::InspectNewestLiveEdge
    );
    assert_eq!(fence.take_pending_inspection(), Some(omitted));
    assert_eq!(
        fence.take_pending_inspection(),
        None,
        "one global commit permits only one bounded inspection"
    );
    assert_eq!(
        fence.observe(omitted),
        GlobalCommitDecision::IgnoredStaleOrDuplicate
    );
    assert_eq!(
        fence.observe(GlobalResponseCommit::new(6, 99)),
        GlobalCommitDecision::IgnoredStaleOrDuplicate,
        "a retired core generation cannot reopen live-edge work"
    );
}

#[test]
fn room_checkpoint_covers_only_its_exact_global_response() {
    let mut fence = GlobalCommitFence::default();

    fence.note_room_checkpoint_advanced(12);
    assert_eq!(
        fence.observe(GlobalResponseCommit::new(7, 11)),
        GlobalCommitDecision::InspectNewestLiveEdge,
        "an N+1 room checkpoint cannot cover an omitted room in response N",
    );
    assert_eq!(
        fence.take_pending_inspection(),
        Some(GlobalResponseCommit::new(7, 11)),
    );
    assert_eq!(
        fence.observe(GlobalResponseCommit::new(7, 12)),
        GlobalCommitDecision::CoveredByRoomCheckpoint,
    );
}

#[test]
fn global_commit_selects_only_the_newest_gap_for_bounded_live_edge_repair() {
    assert_eq!(global_commit_gap_selection(0), GapRepairSelection::None);
    assert_eq!(
        global_commit_gap_selection(4),
        GapRepairSelection::Unprojected {
            ordinal: 3,
            reason: UnprojectedGapReason::LiveEdge,
        },
    );
}

#[test]
fn global_commit_messages_preserve_engine_neutral_identity() {
    let commit = GlobalResponseCommit::new(7, 11);
    let manager = TimelineMessage::AllRoomsResponseCommitted {
        core_generation: commit.core_generation,
        response_sequence: commit.response_sequence,
    };
    assert!(matches!(
        manager,
        TimelineMessage::AllRoomsResponseCommitted {
            core_generation: 7,
            response_sequence: 11,
        }
    ));
    assert!(matches!(
        TimelineActorMessage::GlobalResponseCommitted(commit),
        TimelineActorMessage::GlobalResponseCommitted(GlobalResponseCommit {
            core_generation: 7,
            response_sequence: 11,
        })
    ));
}

#[test]
fn global_commit_inspection_targets_only_active_room_timelines() {
    assert!(is_global_commit_inspection_target(&TimelineKind::Room {
        room_id: "!room:example.org".to_owned(),
    }));
    assert!(!is_global_commit_inspection_target(&TimelineKind::Thread {
        room_id: "!room:example.org".to_owned(),
        root_event_id: "$root:example.org".to_owned(),
    }));
    assert!(!is_global_commit_inspection_target(
        &TimelineKind::Focused {
            room_id: "!room:example.org".to_owned(),
            event_id: "$event:example.org".to_owned(),
        }
    ));
}

#[test]
fn missing_committed_gap_is_reinspected_once_then_closed() {
    let retry_key = (7, 11);
    assert_eq!(
        missing_committed_gap_decision(true, None, retry_key),
        MissingCommittedGapDecision::Retry
    );
    assert_eq!(
        missing_committed_gap_decision(true, Some(retry_key), retry_key),
        MissingCommittedGapDecision::CloseStale
    );
    assert_eq!(
        missing_committed_gap_decision(false, Some(retry_key), retry_key),
        MissingCommittedGapDecision::Noop
    );
}

#[tokio::test]
async fn lagged_observable_projection_wait_is_bounded() {
    assert_eq!(
        wait_for_gap_repair_projection_with_timeout(
            Duration::from_millis(1),
            std::future::pending(),
        )
        .await,
        TimelineGapObservableSettlement::TimedOut
    );
}

#[test]
fn unlocated_gap_has_no_projection_position() {
    assert_eq!(projected_gap_insertion_index(None, None), None);
    assert_eq!(projected_gap_insertion_index(Some(7), None), Some(7));
    assert_eq!(projected_gap_insertion_index(None, Some(7)), Some(8));
}

#[test]
fn projected_gap_identity_is_stable_only_within_the_same_topology_revision() {
    let select = |topology_revision| {
        let projected = [(1, projected_gap_position(topology_revision, 1, 18))];
        select_projected_gap_candidate(&projected, Some((15, 20)), &[])
            .expect("the projected gap intersects the viewport")
    };

    let first = select(7);
    let repeated = select(7);
    let revised = select(8);

    assert_eq!(first.id, repeated.id);
    assert_ne!(first.id, revised.id);
}

#[test]
fn timeline_gap_id_wire_preserves_full_range_projected_identity() {
    let id = projected_gap_id(14_695_981_039_346_656_037, 1);

    let encoded = serde_json::to_string(&id).expect("projected gap id serializes");
    assert_eq!(
        encoded,
        r#"{"topology_revision":"14695981039346656037","ordinal":1}"#
    );
    assert_eq!(
        serde_json::from_str::<TimelineGapId>(&encoded).expect("projected gap id deserializes"),
        id
    );
}

#[test]
fn projected_gap_identity_validates_revision_and_ordinal_before_descriptor_lookup() {
    let selected = projected_gap_id(7, 1);

    assert!(projected_gap_identity_matches_descriptor(selected, 1, 7));
    assert!(!projected_gap_identity_matches_descriptor(selected, 1, 8));
    assert!(!projected_gap_identity_matches_descriptor(selected, 0, 7));
}

#[test]
fn gap_projection_counts_unlocated_sdk_descriptors() {
    let counts = summarize_gap_boundary_presence([
        (false, false),
        (false, false),
        (false, false),
        (false, false),
    ]);

    assert_eq!(
        counts,
        GapBoundaryPresenceCounts {
            both: 0,
            one: 0,
            none: 4,
            projected: 0,
        }
    );
}

#[test]
fn foreground_unlocated_selection_is_distinguished_from_blocked_selection() {
    assert_eq!(
        gap_selection_diagnostic_decision(GapRepairSelection::None, None, true, 4, 0,),
        "foreground_unlocated"
    );
    assert_eq!(
        gap_selection_diagnostic_decision(GapRepairSelection::None, None, false, 4, 0,),
        "blocked"
    );
}

#[test]
fn foreground_unlocated_gap_has_one_action_policy() {
    assert_eq!(
        unlocated_gap_action(true, TimelineGapRepairTrigger::Automatic, 2, 0),
        UnlocatedGapAction::RepairNewest { ordinal: 1 }
    );
    assert_eq!(
        unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 0),
        UnlocatedGapAction::QueueAutomatic
    );
    for action in [
        unlocated_gap_action(false, TimelineGapRepairTrigger::Automatic, 2, 0),
        unlocated_gap_action(false, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 0),
        unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 0, 0),
        unlocated_gap_action(true, TimelineGapRepairTrigger::Automatic, 4, 1),
        unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 1),
    ] {
        assert_eq!(action, UnlocatedGapAction::None);
    }
}

#[test]
fn projected_selection_diagnostic_preserves_candidate_relation() {
    let id = projected_gap_id(7, 1);
    for (relation, expected) in [
        (ProjectedGapRelation::ExplicitVisible, "explicit_visible"),
        (ProjectedGapRelation::IntersectsViewport, "viewport"),
        (ProjectedGapRelation::NearestLiveEdge, "nearest_live_edge"),
    ] {
        assert_eq!(
            gap_selection_diagnostic_decision(
                GapRepairSelection::Projected { id },
                Some(ProjectedGapCandidate { id, relation }),
                true,
                1,
                1,
            ),
            expected
        );
    }
}

#[test]
fn unlocated_gap_diagnostics_are_private_safe() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_timeline_gap_projection(
        4,
        GapBoundaryPresenceCounts {
            both: 0,
            one: 0,
            none: 4,
            projected: 0,
        },
        19,
        true,
        3,
        "idle",
    );
    record_timeline_gap_demand(3, 0, 0, false, "room_selected", "idle");
    record_timeline_gap_selection(TimelineGapSelectionDiagnostic {
        trigger: "cache_gap",
        decision: "foreground_unlocated",
        repair_started: false,
        gap_count: 4,
        projected_gap_count: 0,
        visible_gap_count: 0,
        foreground_demand_active: true,
        foreground_demand_epoch: 3,
        has_live_edge_target: false,
        scheduler_phase: "idle",
    });

    let snapshot = koushi_diagnostics::test_support::detail_snapshot();
    for source in [
        "core.timeline_gap_projection",
        "core.timeline_gap_demand",
        "core.timeline_gap_selection",
    ] {
        let event = &snapshot
            .records
            .iter()
            .rev()
            .find(|record| record.event.source == source)
            .expect("new gap diagnostic")
            .event;
        let debug = format!("{event:?}");
        for forbidden in [
            "room_id",
            "event_id",
            "user_id",
            "gap_id",
            "transaction_id",
            "message",
            "ordinal",
        ] {
            assert!(!debug.contains(forbidden), "{source} leaked {forbidden}");
        }
    }
}

#[test]
fn gap_projection_boundary_diagnostics_correlate_without_private_identifiers() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_timeline_gap_projection_boundary(
        "relay_received",
        "accepted",
        41,
        TimelineGeneration(7),
        historical_causal_projection_operation(13),
        Some(3),
        Some(TimelineBatchId(19)),
        Some(3),
        1,
    );

    let event = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "core.timeline_gap_projection"
                && record.event.stage == "relay_received"
        })
        .expect("projection boundary diagnostic")
        .event;
    let keys = event
        .fields
        .iter()
        .map(|field| field.key)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "outcome",
            "domain",
            "actor_generation",
            "timeline_generation",
            "operation_generation",
            "projection_batch",
            "timeline_batch_id",
            "expected_projection_batch",
            "observed_projection_count",
        ]
    );
    let debug = format!("{event:?}");
    for forbidden in ["room_id", "event_id", "user_id", "gap_id", "message"] {
        assert!(!debug.contains(forbidden));
    }
}

#[test]
fn automatic_repair_prefers_a_gap_intersecting_the_viewport() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
        (2, projected_gap_position(7, 2, 40)),
    ];
    assert_eq!(
        select_projected_gap_id(&projected, Some((15, 20))),
        Some(projected_gap_id(7, 1))
    );
    assert_eq!(
        select_projected_gap_id(&projected, Some((25, 30))),
        Some(projected_gap_id(7, 2))
    );
}

#[test]
fn visible_gap_demand_is_preferred_over_inferred_event_bounds() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
    ];
    let visible_gap_id = projected_gap_id(7, 0);

    assert_eq!(
        select_gap_repair_candidate(
            TimelineGapRepairTrigger::Automatic,
            &projected,
            Some((15, 20)),
            &[visible_gap_id],
            2,
            false,
        ),
        GapRepairSelection::Projected { id: visible_gap_id }
    );
}

#[test]
fn visible_gap_without_event_bounds_wakes_foreground_repair() {
    let projected = vec![(0, projected_gap_position(7, 0, 3))];
    let visible_gap_id = projected_gap_id(7, 0);

    assert_eq!(
        evaluate_gap_repair_viewport_wake(&projected, None, &[visible_gap_id], None,),
        GapRepairViewportWakeDecision::Wake {
            candidate: ProjectedGapCandidate {
                id: visible_gap_id,
                relation: ProjectedGapRelation::ExplicitVisible,
            }
        }
    );
}

#[test]
fn stale_visible_gap_is_ignored_and_requests_fresh_inspection() {
    let projected = vec![(0, projected_gap_position(7, 0, 3))];
    let stale_visible_gap_id = projected_gap_id(8, 0);

    assert_eq!(
        evaluate_gap_repair_viewport_wake(&projected, Some((1, 5)), &[stale_visible_gap_id], None,),
        GapRepairViewportWakeDecision::WakeStaleVisibleDemand
    );
    assert_eq!(
        select_gap_repair_candidate(
            TimelineGapRepairTrigger::Automatic,
            &projected,
            Some((1, 5)),
            &[stale_visible_gap_id],
            1,
            false,
        ),
        GapRepairSelection::None
    );
}

#[test]
fn stale_visible_gap_does_not_suppress_independent_live_edge_fallback() {
    let projected = vec![(0, projected_gap_position(7, 0, 3))];

    assert_eq!(
        select_gap_repair_candidate(
            TimelineGapRepairTrigger::LiveEdge,
            &projected,
            Some((1, 5)),
            &[projected_gap_id(8, 0)],
            2,
            true,
        ),
        GapRepairSelection::Unprojected {
            ordinal: 1,
            reason: UnprojectedGapReason::LiveEdge,
        }
    );
}

#[test]
fn viewport_wake_requests_inspection_when_projected_candidate_changes() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
    ];

    assert_eq!(
        evaluate_gap_repair_viewport_wake(&projected, Some((15, 20)), &[], None),
        GapRepairViewportWakeDecision::Wake {
            candidate: ProjectedGapCandidate {
                id: projected_gap_id(7, 1),
                relation: ProjectedGapRelation::IntersectsViewport,
            },
        }
    );
}

#[test]
fn viewport_wake_ignores_repeated_observation_for_same_candidate() {
    let projected = vec![(0, projected_gap_position(7, 0, 8))];
    let previous = ProjectedGapCandidate {
        id: projected_gap_id(7, 0),
        relation: ProjectedGapRelation::IntersectsViewport,
    };

    assert_eq!(
        evaluate_gap_repair_viewport_wake(&projected, Some((5, 10)), &[], Some(previous)),
        GapRepairViewportWakeDecision::IdleUnchangedCandidate {
            candidate: previous,
        }
    );
}

#[test]
fn viewport_wake_requests_again_when_viewport_selects_another_gap() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
    ];
    let previous = ProjectedGapCandidate {
        id: projected_gap_id(7, 1),
        relation: ProjectedGapRelation::IntersectsViewport,
    };

    assert_eq!(
        evaluate_gap_repair_viewport_wake(&projected, Some((1, 5)), &[], Some(previous)),
        GapRepairViewportWakeDecision::Wake {
            candidate: ProjectedGapCandidate {
                id: projected_gap_id(7, 0),
                relation: ProjectedGapRelation::IntersectsViewport,
            },
        }
    );
}

#[test]
fn internal_gap_projection_relay_fences_actor_timeline_repair_and_batch() {
    let expected = GapProjectionRelayed {
        actor_generation: 9,
        timeline_generation: TimelineGeneration(3),
        repair_generation: 11,
        minimum_batch_id: TimelineBatchId(5),
    };
    assert!(gap_projection_relay_is_current(
        expected,
        9,
        TimelineGeneration(3),
        11,
        TimelineBatchId(5),
    ));
    for stale in [
        GapProjectionRelayed {
            actor_generation: 8,
            ..expected
        },
        GapProjectionRelayed {
            timeline_generation: TimelineGeneration(2),
            ..expected
        },
        GapProjectionRelayed {
            repair_generation: 10,
            ..expected
        },
        GapProjectionRelayed {
            minimum_batch_id: TimelineBatchId(4),
            ..expected
        },
    ] {
        assert!(!gap_projection_relay_is_current(
            stale,
            9,
            TimelineGeneration(3),
            11,
            TimelineBatchId(5),
        ));
    }
}

#[test]
fn observe_viewport_wakes_only_after_projected_candidate_changes() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
    ];
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.replace_projected_gaps(projected, Some((15, 20)), &[]);

    assert!(matches!(
        tracker.evaluate_viewport_wake(Some((15, 20)), &[]),
        GapRepairViewportWakeDecision::IdleUnchangedCandidate { .. }
    ));
    assert_eq!(
        tracker.evaluate_viewport_wake(Some((1, 5)), &[]),
        GapRepairViewportWakeDecision::Wake {
            candidate: ProjectedGapCandidate {
                id: projected_gap_id(7, 0),
                relation: ProjectedGapRelation::IntersectsViewport,
            }
        }
    );
}

#[test]
fn viewport_wake_evaluation_diagnostics_are_private_safe() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_timeline_gap_repair_evaluation("wake", 2, 1, true, true, "awaiting_relay");

    let record = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "core.timeline_gap_repair" && record.event.stage == "evaluation"
        })
        .expect("viewport wake evaluation diagnostic");
    let keys = record
        .event
        .fields
        .iter()
        .map(|field| field.key)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "trigger",
            "decision",
            "projected_gap_count",
            "visible_gap_count",
            "visible_gap_validated",
            "candidate_changed",
            "scheduler_phase",
        ]
    );
    let debug = format!("{:?}", record.event);
    for forbidden in ["room_id", "event_id", "user_id", "gap_id", "message"] {
        assert!(!debug.contains(forbidden));
    }
}

#[test]
fn gap_repair_wake_is_retained_across_inspection_order() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
    ];
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.replace_projected_gaps(projected, Some((15, 20)), &[]);

    assert!(matches!(
        tracker.evaluate_viewport_wake(Some((1, 5)), &[]),
        GapRepairViewportWakeDecision::Wake { .. }
    ));
    tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
    assert_eq!(tracker.begin_pending_inspection(false), None);
    let (first_serial, _) = tracker
        .begin_pending_inspection(true)
        .expect("projection ACK releases the queued viewport wake");

    assert!(matches!(
        tracker.evaluate_viewport_wake(Some((15, 20)), &[]),
        GapRepairViewportWakeDecision::Wake { .. }
    ));
    tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
    assert_eq!(tracker.begin_pending_inspection(true), None);
    assert!(tracker.finish_work(first_serial));
    let (second_serial, _) = tracker
        .begin_pending_inspection(true)
        .expect("active inspection completion releases the changed candidate");
    assert!(tracker.finish_work(second_serial));

    assert_eq!(
        timeline_gap_repair_budget(
            TimelineGapRepairTrigger::Automatic,
            AccountWorkKind::OffscreenGapRepair
        )
        .cached_chunk_limit,
        1
    );
}

#[test]
fn candidate_wake_queued_during_repair_is_available_after_terminal_release() {
    let projected = vec![
        (0, projected_gap_position(7, 0, 3)),
        (1, projected_gap_position(7, 1, 18)),
    ];
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.replace_projected_gaps(projected, Some((1, 5)), &[]);
    let repair_serial = tracker
        .begin_repair(2)
        .expect("the initial repair should own the scheduler");

    assert!(matches!(
        tracker.evaluate_viewport_wake(Some((15, 20)), &[]),
        GapRepairViewportWakeDecision::Wake { .. }
    ));
    tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
    assert_eq!(tracker.begin_pending_inspection(true), None);

    assert!(tracker.finish_work(repair_serial));
    assert!(tracker.begin_pending_inspection(true).is_some());
}

#[test]
fn repeated_gap_repair_evaluation_signature_is_deduplicated() {
    let signature = GapRepairEvaluationDiagnosticSignature {
        decision: "idle_unchanged",
        projected_gap_count: 2,
        visible_gap_count: 1,
        visible_gap_validated: true,
        candidate_changed: false,
        scheduler_phase: "idle",
    };
    let mut previous = None;

    assert!(should_record_gap_repair_evaluation(
        &mut previous,
        signature
    ));
    assert!(!should_record_gap_repair_evaluation(
        &mut previous,
        signature
    ));
    assert!(should_record_gap_repair_evaluation(
        &mut previous,
        GapRepairEvaluationDiagnosticSignature {
            scheduler_phase: "active",
            ..signature
        }
    ));
}

#[test]
fn automatic_and_manual_repair_use_separate_cache_budgets() {
    // The event bound comes from the work policy; only the cache budget
    // varies by trigger.
    for (trigger, work_kind) in [
        (
            TimelineGapRepairTrigger::Automatic,
            AccountWorkKind::OffscreenGapRepair,
        ),
        (
            TimelineGapRepairTrigger::LiveEdge,
            AccountWorkKind::OffscreenGapRepair,
        ),
        (
            TimelineGapRepairTrigger::Manual,
            AccountWorkKind::VisibleGapRepair,
        ),
    ] {
        assert_eq!(
            timeline_gap_repair_budget(trigger, work_kind),
            MatrixTimelineGapRepairBudget {
                event_limit: work_kind.policy().batch_limit,
                cached_chunk_limit: 1,
            }
        );
    }
    assert_eq!(
        timeline_gap_repair_budget(
            TimelineGapRepairTrigger::LiveTailSnapshot,
            AccountWorkKind::OffscreenGapRepair
        )
        .cached_chunk_limit,
        0,
        "live-tail snapshots must not load cached chunks"
    );
}

#[test]
fn gap_repair_work_kind_follows_reported_visibility() {
    use super::{ProjectedGapCandidate, ProjectedGapRelation};
    let gap_id = TimelineGapId {
        topology_revision: 1,
        ordinal: 0,
    };
    for relation in [
        ProjectedGapRelation::ExplicitVisible,
        ProjectedGapRelation::IntersectsViewport,
    ] {
        assert_eq!(
            gap_repair_work_kind(
                TimelineGapRepairTrigger::Automatic,
                Some(ProjectedGapCandidate {
                    id: gap_id,
                    relation
                })
            ),
            AccountWorkKind::VisibleGapRepair
        );
    }
    assert_eq!(
        gap_repair_work_kind(
            TimelineGapRepairTrigger::Automatic,
            Some(ProjectedGapCandidate {
                id: gap_id,
                relation: ProjectedGapRelation::NearestLiveEdge
            })
        ),
        AccountWorkKind::OffscreenGapRepair
    );
    assert_eq!(
        gap_repair_work_kind(TimelineGapRepairTrigger::LiveEdge, None),
        AccountWorkKind::OffscreenGapRepair,
        "live-edge repair for the selected room stays background"
    );
    assert_eq!(
        gap_repair_work_kind(TimelineGapRepairTrigger::Manual, None),
        AccountWorkKind::VisibleGapRepair,
        "an explicitly requested repair is foreground even without a candidate"
    );
    // Background repair must never outrank a send or visible pagination.
    assert!(
        AccountWorkKind::OffscreenGapRepair.policy().priority
            > AccountWorkKind::ExplicitPagination.policy().priority
    );
    assert!(
        AccountWorkKind::VisibleGapRepair.policy().priority
            > AccountWorkKind::MessageSend.policy().priority
    );
}

#[test]
fn trigger_priority_keeps_live_edge_between_viewport_and_manual() {
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
    tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
    assert!(matches!(
        tracker.begin_pending_inspection(true),
        Some((_, TimelineGapRepairTrigger::LiveEdge))
    ));

    let mut tracker = TimelineGapRepairTracker::default();
    tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
    tracker.queue_inspection(TimelineGapRepairTrigger::Manual);
    assert!(matches!(
        tracker.begin_pending_inspection(true),
        Some((_, TimelineGapRepairTrigger::Manual))
    ));

    let mut tracker = TimelineGapRepairTracker::default();
    tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
    tracker.queue_inspection(TimelineGapRepairTrigger::LiveTailSnapshot);
    assert!(matches!(
        tracker.begin_pending_inspection(true),
        Some((_, TimelineGapRepairTrigger::LiveTailSnapshot))
    ));

    let mut tracker = TimelineGapRepairTracker::default();
    tracker.queue_inspection(TimelineGapRepairTrigger::LiveTailSnapshot);
    tracker.queue_inspection(TimelineGapRepairTrigger::Manual);
    assert!(matches!(
        tracker.begin_pending_inspection(true),
        Some((_, TimelineGapRepairTrigger::Manual))
    ));
}

#[test]
fn live_tail_snapshot_observes_projected_gaps_without_repairing_them() {
    let projected = vec![(0, projected_gap_position(7, 0, 0))];
    assert_eq!(
        select_gap_repair_candidate(
            TimelineGapRepairTrigger::LiveTailSnapshot,
            &projected,
            Some((0, 0)),
            &[],
            1,
            true,
        ),
        GapRepairSelection::None,
    );
}

#[test]
fn final_live_tail_projection_batch_queues_one_snapshot_instead_of_live_edge() {
    assert_eq!(
        post_diff_gap_inspection_trigger(true, true, true),
        Some(TimelineGapRepairTrigger::LiveTailSnapshot),
        "the exact final live-tail batch must publish one observation instead of leaving a repair-capable LiveEdge request behind"
    );
    assert_eq!(
        post_diff_gap_inspection_trigger(true, false, true),
        None,
        "an intermediate live-tail batch must not queue automatic or live-edge repair",
    );
    assert_eq!(
        post_diff_gap_inspection_trigger(
            true,
            live_tail_completion_requires_snapshot(MatrixLiveTailRefreshOutcome::Failed),
            true,
        ),
        None,
        "a failed live-tail completion must not create an observation snapshot",
    );
    assert_eq!(
        post_diff_gap_inspection_trigger(false, false, true),
        Some(TimelineGapRepairTrigger::LiveEdge),
    );
    assert_eq!(
        post_diff_gap_inspection_trigger(false, false, false),
        Some(TimelineGapRepairTrigger::Automatic),
    );
}

#[test]
fn live_edge_fallback_selects_only_the_newest_unprojected_gap() {
    assert_eq!(
        select_gap_repair_candidate(TimelineGapRepairTrigger::Automatic, &[], None, &[], 4, true,),
        GapRepairSelection::None,
    );
    assert_eq!(
        select_gap_repair_candidate(TimelineGapRepairTrigger::LiveEdge, &[], None, &[], 4, false,),
        GapRepairSelection::None,
    );
    assert_eq!(
        select_gap_repair_candidate(TimelineGapRepairTrigger::LiveEdge, &[], None, &[], 4, true,),
        GapRepairSelection::Unprojected {
            ordinal: 3,
            reason: UnprojectedGapReason::LiveEdge,
        },
    );
}

#[test]
fn live_edge_target_change_rearms_a_bounded_attempt() {
    let mut tracker = TimelineGapRepairTracker::default();
    assert!(tracker.observe_live_edge_target(Some("$owner-a".to_owned())));
    assert!(!tracker.observe_live_edge_target(Some("$owner-a".to_owned())));

    for _ in 0..MAX_LIVE_EDGE_GAP_REPAIR_BATCHES {
        assert!(
            tracker
                .record_batch(TimelineGapRepairTrigger::LiveEdge)
                .is_some()
        );
    }
    assert!(!tracker.can_start_batch(TimelineGapRepairTrigger::LiveEdge));

    assert!(tracker.observe_live_edge_target(Some("$owner-b".to_owned())));
    assert!(tracker.can_start_batch(TimelineGapRepairTrigger::LiveEdge));
}

#[test]
fn unchanged_live_edge_topology_after_a_batch_is_no_progress() {
    let mut tracker = TimelineGapRepairTracker::default();
    let selection = LiveEdgeGapSelection {
        topology_revision: 17,
        ordinal: 3,
    };

    assert_eq!(
        tracker.evaluate_live_edge_selection(selection),
        LiveEdgeSelectionDecision::Repair,
    );
    assert!(
        tracker
            .record_batch(TimelineGapRepairTrigger::LiveEdge)
            .is_some()
    );
    assert_eq!(
        tracker.evaluate_live_edge_selection(selection),
        LiveEdgeSelectionDecision::NoProgress,
    );
}

#[test]
fn live_edge_zero_progress_outcomes_terminate() {
    for outcome in [
        MatrixTimelineGapRepairOutcome::Stale,
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 0,
        },
        MatrixTimelineGapRepairOutcome::Progress { events: 0 },
    ] {
        assert!(!timeline_gap_repair_made_progress(&outcome));
    }
    for outcome in [
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 1,
        },
        MatrixTimelineGapRepairOutcome::Progress { events: 1 },
        MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 0 },
        MatrixTimelineGapRepairOutcome::StartReached { events: 0 },
    ] {
        assert!(timeline_gap_repair_made_progress(&outcome));
    }
}

#[test]
fn gap_repair_result_diagnostics_preserve_sdk_outcome_and_progress_counts() {
    let cases = [
        (
            Ok(MatrixTimelineGapRepairResult {
                outcome: MatrixTimelineGapRepairOutcome::Deferred {
                    cached_chunks_loaded: 3,
                },
                last_projection_batch: Some(2),
            }),
            ("deferred", 0, 3, true, true),
        ),
        (
            Ok(MatrixTimelineGapRepairResult {
                outcome: MatrixTimelineGapRepairOutcome::Progress { events: 17 },
                last_projection_batch: Some(1),
            }),
            ("progress", 17, 0, true, true),
        ),
        (
            Ok(MatrixTimelineGapRepairResult {
                outcome: MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 5 },
                last_projection_batch: None,
            }),
            ("boundaries_joined", 5, 0, false, true),
        ),
        (
            Ok(MatrixTimelineGapRepairResult {
                outcome: MatrixTimelineGapRepairOutcome::StartReached { events: 4 },
                last_projection_batch: None,
            }),
            ("start_reached", 4, 0, false, true),
        ),
        (
            Ok(MatrixTimelineGapRepairResult {
                outcome: MatrixTimelineGapRepairOutcome::Stale,
                last_projection_batch: None,
            }),
            ("stale", 0, 0, false, false),
        ),
        (
            Ok(MatrixTimelineGapRepairResult {
                outcome: MatrixTimelineGapRepairOutcome::Failed,
                last_projection_batch: None,
            }),
            ("failed", 0, 0, false, false),
        ),
        (
            Err(MatrixTimelineGapError::Sdk),
            ("error", 0, 0, false, false),
        ),
    ];

    for (result, expected) in cases {
        let diagnostic = timeline_gap_repair_result_diagnostic(&result);
        assert_eq!(
            (
                diagnostic.outcome,
                diagnostic.events,
                diagnostic.cached_chunks_loaded,
                diagnostic.has_projection_batch,
                diagnostic.made_progress,
            ),
            expected,
        );
    }
}

#[test]
fn repaired_live_edge_does_not_continue_into_an_unrelated_historical_gap() {
    assert_eq!(
        gap_repair_continuation_trigger(
            TimelineGapRepairTrigger::LiveEdge,
            true,
            &MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 3 },
        ),
        TimelineGapRepairTrigger::Automatic,
    );
    assert_eq!(
        gap_repair_continuation_trigger(
            TimelineGapRepairTrigger::LiveEdge,
            true,
            &MatrixTimelineGapRepairOutcome::Progress { events: 3 },
        ),
        TimelineGapRepairTrigger::LiveEdge,
    );
    assert_eq!(
        gap_repair_continuation_trigger(
            TimelineGapRepairTrigger::LiveEdge,
            false,
            &MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 3 },
        ),
        TimelineGapRepairTrigger::LiveEdge,
        "repairing a projected gap must preserve the live-edge intent"
    );
}

#[test]
fn actor_fixture_recovers_relation_bounded_live_edge_after_internal_relay() {
    // The raw newest boundary is an edit/reaction and therefore has no
    // standalone projected row. The rendered owner still supplies the
    // actor-private live-edge target.
    let actor_generation = 7;
    let timeline_generation = TimelineGeneration(3);
    let projection_batch = 1;
    let rendered_batch_id = TimelineBatchId(41);
    let older = event_item("$older:test", "older");
    let missing = event_item("$missing:test", "missing");
    let newer_owner = event_item("$owner:test", "newer");
    let mut rendered_items = vec![older.clone(), newer_owner.clone()];
    let mut tracker = TimelineGapRepairTracker::default();
    let mut correlation = TimelineGapProjectionCorrelation::default();

    assert!(tracker.observe_live_edge_target(rendered_live_edge_target(&rendered_items)));
    tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
    assert_eq!(
        tracker.begin_pending_inspection(false),
        None,
        "the initial actor projection must commit before inspection"
    );
    let (inspection_serial, trigger) = tracker
        .begin_pending_inspection(true)
        .expect("initial actor projection releases live-edge inspection");
    assert_eq!(trigger, TimelineGapRepairTrigger::LiveEdge);
    assert!(tracker.finish_work(inspection_serial));

    let projected_relation_boundaries = Vec::new();
    assert_eq!(
        select_gap_repair_candidate(
            trigger,
            &projected_relation_boundaries,
            None,
            &[],
            3,
            tracker.has_live_edge_target(),
        ),
        GapRepairSelection::Unprojected {
            ordinal: 2,
            reason: UnprojectedGapReason::LiveEdge,
        },
    );

    let repair_serial = tracker.begin_repair(3).expect("repair owns scheduler");
    let repair_operation = historical_causal_projection_operation(repair_serial);
    correlation.begin(actor_generation, repair_operation);

    // Model the SDK relay publication carrying the repair correlation tag.
    // A duplicate delivery is included deliberately: the same display
    // normalization used by TimelineActor/WebView must retain one row.
    apply_timeline_diffs_to_display_items(
        &mut rendered_items,
        &[
            TimelineDiff::Insert {
                index: 1,
                item: missing.clone(),
            },
            TimelineDiff::Insert {
                index: 1,
                item: missing.clone(),
            },
        ],
    );
    assert_eq!(
        correlation.observe(
            CausalProjectionId {
                actor_generation,
                operation: repair_operation,
                projection_batch,
            },
            rendered_batch_id,
        ),
        None,
        "publication alone cannot continue before SDK completion"
    );
    assert_eq!(
        correlation.complete(actor_generation, repair_operation, Some(projection_batch)),
        TimelineGapProjectionCompletion::Ready(rendered_batch_id),
    );
    assert!(tracker.finish_work(repair_serial));
    assert_eq!(
        tracker.record_batch(trigger),
        Some(1),
        "one bounded live-edge repair batch is recorded"
    );

    // Once that newest gap joins, reinspection uses ordinary automatic
    // policy, so the two unrelated unprojected historical gaps stay idle.
    let continuation = gap_repair_continuation_trigger(
        trigger,
        true,
        &MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 1 },
    );
    assert_eq!(continuation, TimelineGapRepairTrigger::Automatic);
    tracker.queue_inspection(continuation);
    let relayed = GapProjectionRelayed {
        actor_generation,
        timeline_generation,
        repair_generation: repair_serial,
        minimum_batch_id: rendered_batch_id,
    };
    assert!(gap_projection_relay_is_current(
        relayed,
        actor_generation,
        timeline_generation,
        repair_serial,
        rendered_batch_id,
    ));
    let (continuation_serial, continuation) = tracker
        .begin_pending_inspection(true)
        .expect("the exact internal relay releases continuation");
    assert_eq!(
        select_gap_repair_candidate(
            continuation,
            &projected_relation_boundaries,
            None,
            &[],
            2,
            true,
        ),
        GapRepairSelection::None,
    );
    assert!(tracker.finish_work(continuation_serial));

    assert_eq!(rendered_items, vec![older, missing.clone(), newer_owner]);
    assert_eq!(
        rendered_items
            .iter()
            .filter(|item| item.id == missing.id)
            .count(),
        1,
        "the repaired interval is projected exactly once"
    );
}

#[test]
fn live_edge_diagnostic_trigger_is_private_safe() {
    assert_eq!(
        timeline_gap_repair_trigger_token(TimelineGapRepairTrigger::LiveEdge),
        "live_edge"
    );
}

#[test]
fn subscription_inspection_waits_for_initial_projection_commit() {
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
    assert_eq!(tracker.begin_pending_inspection(false), None);
    assert!(tracker.has_pending_inspection());
    assert!(matches!(
        tracker.begin_pending_inspection(true),
        Some((_, TimelineGapRepairTrigger::Automatic))
    ));
}

#[test]
fn relay_overflow_clears_obsolete_gap_correlation_and_requeues_live_edge() {
    let actor_generation = 9;
    let mut tracker = TimelineGapRepairTracker::default();
    let repair_generation = tracker.begin_repair(1).expect("repair owns scheduler");
    let mut correlation = TimelineGapProjectionCorrelation::default();
    correlation.begin(
        actor_generation,
        historical_causal_projection_operation(repair_generation),
    );
    let mut pending = Some(PendingTimelineGapProjection {
        trigger: TimelineGapRepairTrigger::LiveEdge,
        repair_generation,
        gap_count: 1,
        batches_processed: 1,
    });

    assert!(recover_obsolete_gap_settlement(
        &mut correlation,
        &mut pending,
        &mut tracker,
        actor_generation,
        repair_generation,
        TimelineGapRepairTrigger::LiveEdge,
    ));
    assert!(!correlation.is_pending());
    assert!(pending.is_none());
    let (_, trigger) = tracker
        .begin_pending_inspection(true)
        .expect("overflow recovery must release and requeue LiveEdge");
    assert_eq!(trigger, TimelineGapRepairTrigger::LiveEdge);
}

#[test]
fn stale_prior_actor_gap_projection_is_removed_from_every_relay_batch() {
    let current_actor_generation = 9;
    let stale = CausalProjectionId {
        actor_generation: current_actor_generation - 1,
        operation: historical_causal_projection_operation(11),
        projection_batch: 1,
    };
    let current = CausalProjectionId {
        actor_generation: current_actor_generation,
        operation: historical_causal_projection_operation(12),
        projection_batch: 1,
    };

    for _ in 0..3 {
        let mut batch = TimelineRelayBatch {
            generation: TimelineGeneration(4),
            diffs: Vec::new(),
            thread_attention_provenance: ThreadAttentionBatchProvenance::default(),
            gap_repair_projections: BTreeSet::from([stale, current]),
        };
        batch.retain_gap_repair_projections_for_actor(current_actor_generation);
        assert_eq!(
            batch.gap_repair_projections,
            BTreeSet::from([current]),
            "every relay batch must drop superseded descriptors and retain current identity"
        );
    }
}

#[test]
fn timeline_gap_repair_tracker_coalesces_and_rejects_stale_completions() {
    let mut tracker = TimelineGapRepairTracker::default();
    let first = tracker.begin_inspection().expect("first inspection");
    assert!(tracker.begin_inspection().is_none());
    assert!(!tracker.finish_work(first.wrapping_add(1)));
    assert!(tracker.finish_work(first));

    let repair = tracker.begin_repair(2).expect("repair starts");
    assert_eq!(tracker.gap_count, 2);
    assert!(tracker.begin_repair(2).is_none());
    assert!(tracker.finish_work(repair));
}

#[test]
fn historical_projection_serial_exhaustion_never_reuses_one() {
    let actor_generation = 9;
    let mut correlation = TimelineGapProjectionCorrelation::default();
    let prior_operation = historical_causal_projection_operation(CAUSAL_PROJECTION_SERIAL_MAX);
    correlation.begin(actor_generation, prior_operation);
    assert_eq!(
        correlation.complete(actor_generation, prior_operation, Some(1)),
        TimelineGapProjectionCompletion::Pending,
    );

    let mut tracker = TimelineGapRepairTracker {
        next_serial: CAUSAL_PROJECTION_SERIAL_MAX,
        ..TimelineGapRepairTracker::default()
    };
    assert_eq!(tracker.begin_repair(1), None);
    assert_eq!(tracker.begin_repair(1), None, "exhaustion cannot busy-loop");
    assert!(tracker.active_serial.is_none());
    assert_eq!(tracker.next_serial, CAUSAL_PROJECTION_SERIAL_MAX);
    assert_eq!(
        correlation.observe(
            CausalProjectionId {
                actor_generation,
                operation: historical_causal_projection_operation(1),
                projection_batch: 1,
            },
            TimelineBatchId(8),
        ),
        None,
        "serial one from a hypothetical wrap cannot cross-complete the prior identity",
    );
    assert!(correlation.is_pending());
}

#[test]
fn gap_repair_progress_budget_allows_cache_reveal_beyond_total_batch_count() {
    let mut tracker = TimelineGapRepairTracker::default();
    let id = projected_gap_id(7, 1);
    let demand_revision = 11;
    assert!(tracker.admit_gap_attempt(id, demand_revision).is_some());

    for expected in 1..=MAX_TIMELINE_GAP_REPAIR_BATCHES + 1 {
        let outcome = MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 1,
        };
        assert!(timeline_gap_repair_made_progress(&outcome));
        assert_eq!(tracker.attempt_gap_id, Some(id));
        assert_eq!(tracker.attempt_demand_revision, Some(demand_revision));
        assert!(
            tracker.can_start_batch(TimelineGapRepairTrigger::Automatic),
            "cache reveal batch {expected} must remain admissible"
        );
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(expected)
        );
        tracker.record_batch_outcome(&outcome);
        assert_eq!(tracker.consecutive_no_progress_batches, 0);
    }
    assert_eq!(
        tracker.batches_processed,
        MAX_TIMELINE_GAP_REPAIR_BATCHES + 1
    );
}

#[test]
fn gap_repair_attempt_diagnostics_classify_attempt_resets() {
    let mut tracker = TimelineGapRepairTracker::default();

    let initial = tracker
        .admit_gap_attempt(projected_gap_id(7, 1), 11)
        .expect("initial gap attempt is admitted");
    assert_eq!(initial.attempt_number, 1);
    assert_eq!(initial.reason, TimelineGapAttemptResetReason::Initial);
    assert!(!initial.topology_changed);
    assert!(!initial.ordinal_changed);
    assert!(!initial.demand_changed);
    assert_eq!(initial.reason.as_str(), "initial");

    let topology = tracker
        .admit_gap_attempt(projected_gap_id(8, 1), 11)
        .expect("topology change is admitted");
    assert_eq!(topology.attempt_number, 2);
    assert_eq!(topology.reason, TimelineGapAttemptResetReason::Topology);
    assert!(topology.topology_changed);
    assert!(!topology.ordinal_changed);
    assert!(!topology.demand_changed);
    assert_eq!(topology.reason.as_str(), "topology");

    let ordinal = tracker
        .admit_gap_attempt(projected_gap_id(8, 2), 11)
        .expect("ordinal change is admitted");
    assert_eq!(ordinal.attempt_number, 3);
    assert_eq!(ordinal.reason, TimelineGapAttemptResetReason::Ordinal);
    assert!(!ordinal.topology_changed);
    assert!(ordinal.ordinal_changed);
    assert!(!ordinal.demand_changed);
    assert_eq!(ordinal.reason.as_str(), "ordinal");

    let demand = tracker
        .admit_gap_attempt(projected_gap_id(8, 2), 12)
        .expect("explicit demand change is admitted");
    assert_eq!(demand.attempt_number, 4);
    assert_eq!(demand.reason, TimelineGapAttemptResetReason::Demand);
    assert!(!demand.topology_changed);
    assert!(!demand.ordinal_changed);
    assert!(demand.demand_changed);
    assert_eq!(demand.reason.as_str(), "demand");

    assert!(
        tracker
            .admit_gap_attempt(projected_gap_id(8, 2), 12)
            .is_none()
    );
}

#[test]
fn gap_repair_attempt_diagnostics_emit_once_per_changed_admission() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let demand_revision = 9_004_001;
    let mut tracker = TimelineGapRepairTracker::default();

    assert!(admit_and_record_timeline_gap_repair_attempt(
        &mut tracker,
        projected_gap_id(7, 1),
        demand_revision,
    ));
    assert!(admit_and_record_timeline_gap_repair_attempt(
        &mut tracker,
        projected_gap_id(8, 1),
        demand_revision,
    ));
    assert!(admit_and_record_timeline_gap_repair_attempt(
        &mut tracker,
        projected_gap_id(8, 2),
        demand_revision,
    ));
    assert!(admit_and_record_timeline_gap_repair_attempt(
        &mut tracker,
        projected_gap_id(8, 2),
        demand_revision + 1,
    ));
    assert!(!admit_and_record_timeline_gap_repair_attempt(
        &mut tracker,
        projected_gap_id(8, 2),
        demand_revision + 1,
    ));

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let admissions = records[diagnostic_start..]
        .iter()
        .filter(|record| {
            record.event.source == "core.timeline_gap_repair"
                && record.event.stage == "attempt_admitted"
                && record.event.fields.iter().any(|field| {
                    field.key == "demand_revision"
                        && matches!(
                            field.value,
                            koushi_diagnostics::DiagnosticValue::Count(value)
                                if value == demand_revision || value == demand_revision + 1
                        )
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(admissions.len(), 4);
    for reason in ["initial", "topology", "ordinal", "demand"] {
        assert_eq!(
            admissions
                .iter()
                .filter(|record| {
                    record.event.fields.iter().any(|field| {
                        field.key == "reset_reason"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token(reason)
                    })
                })
                .count(),
            1,
            "changed admission must emit reset reason {reason} exactly once",
        );
    }
}

#[test]
fn gap_repair_attempt_diagnostics_emit_one_budget_update_per_sdk_result() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let demand_revision = 9_004_101;
    let mut tracker = TimelineGapRepairTracker::default();
    tracker
        .admit_gap_attempt(projected_gap_id(7, 1), demand_revision)
        .expect("initial gap attempt is admitted");
    let outcomes = [
        MatrixTimelineGapRepairOutcome::Stale,
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 0,
        },
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 1,
        },
        MatrixTimelineGapRepairOutcome::Failed,
        MatrixTimelineGapRepairOutcome::Progress { events: 0 },
        MatrixTimelineGapRepairOutcome::Progress { events: 1 },
        MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 0 },
        MatrixTimelineGapRepairOutcome::StartReached { events: 0 },
    ];

    for (index, outcome) in outcomes.into_iter().enumerate() {
        record_timeline_gap_repair_result(
            &mut tracker,
            index as u64 + 1,
            TimelineGapRepairTrigger::Automatic,
            &Ok(MatrixTimelineGapRepairResult {
                outcome,
                last_projection_batch: None,
            }),
        );
        assert_eq!(
            timeline_gap_repair_diagnostic_count_since(
                diagnostic_start,
                "budget_updated",
                demand_revision,
            ),
            index + 1,
            "each successful SDK result must emit exactly one budget update",
        );
    }

    record_timeline_gap_repair_result(
        &mut tracker,
        outcomes.len() as u64 + 1,
        TimelineGapRepairTrigger::Automatic,
        &Err(MatrixTimelineGapError::Sdk),
    );
    assert_eq!(
        timeline_gap_repair_diagnostic_count_since(
            diagnostic_start,
            "budget_updated",
            demand_revision,
        ),
        outcomes.len() + 1,
        "the SDK error result must emit exactly one budget update",
    );
}

#[test]
fn gap_repair_progress_budget_rejects_thirty_third_consecutive_noop() {
    let mut tracker = TimelineGapRepairTracker::default();
    let id = projected_gap_id(7, 1);
    let demand_revision = 11;
    assert!(tracker.admit_gap_attempt(id, demand_revision).is_some());

    for expected in 1..=MAX_TIMELINE_GAP_REPAIR_BATCHES {
        let outcome = MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 0,
        };
        assert!(!timeline_gap_repair_made_progress(&outcome));
        assert!(tracker.can_start_batch(TimelineGapRepairTrigger::Automatic));
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(expected)
        );
        tracker.record_batch_outcome(&outcome);
        assert_eq!(tracker.consecutive_no_progress_batches, expected);
    }
    assert!(!tracker.can_start_batch(TimelineGapRepairTrigger::Automatic));
    assert_eq!(
        tracker.record_batch(TimelineGapRepairTrigger::Automatic),
        None
    );
    assert_eq!(tracker.attempt_gap_id, Some(id));
    assert_eq!(tracker.attempt_demand_revision, Some(demand_revision));
    assert_eq!(
        tracker.consecutive_no_progress_batches,
        MAX_TIMELINE_GAP_REPAIR_BATCHES
    );
}

#[test]
fn gap_repair_sdk_error_budget_rejects_thirty_third_consecutive_error() {
    let mut tracker = TimelineGapRepairTracker::default();
    let id = projected_gap_id(7, 1);
    let demand_revision = 11;
    assert!(tracker.admit_gap_attempt(id, demand_revision).is_some());

    for expected in 1..=MAX_TIMELINE_GAP_REPAIR_BATCHES {
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(expected)
        );
        tracker.record_batch_error();
        assert_eq!(tracker.consecutive_no_progress_batches, expected);
    }
    assert_eq!(
        tracker.record_batch(TimelineGapRepairTrigger::Automatic),
        None
    );
    assert_eq!(tracker.attempt_gap_id, Some(id));
    assert_eq!(tracker.attempt_demand_revision, Some(demand_revision));

    assert!(
        tracker
            .admit_gap_attempt(projected_gap_id(8, 1), demand_revision)
            .is_some()
    );
    tracker.batches_processed = u32::MAX;
    assert_eq!(
        tracker.record_batch(TimelineGapRepairTrigger::Automatic),
        Some(u32::MAX)
    );
    assert_eq!(tracker.batches_processed, u32::MAX);
}

#[test]
fn gap_repair_budget_is_scoped_without_resetting_repeated_demand() {
    let mut tracker = TimelineGapRepairTracker::default();
    let id = projected_gap_id(7, 1);

    assert!(tracker.admit_gap_attempt(id, 11).is_some());
    assert_eq!(
        tracker.record_batch(TimelineGapRepairTrigger::Automatic),
        Some(1)
    );

    assert!(tracker.admit_gap_attempt(id, 11).is_none());
    assert_eq!(tracker.attempt_gap_id, Some(id));
    assert_eq!(tracker.batches_processed, 1);
}

#[test]
fn gap_repair_budget_is_scoped_to_topology_revision() {
    let mut tracker = TimelineGapRepairTracker::default();
    let first = projected_gap_id(7, 1);
    let revised = projected_gap_id(8, 1);

    assert!(tracker.admit_gap_attempt(first, 11).is_some());
    assert!(
        tracker
            .record_batch(TimelineGapRepairTrigger::LiveEdge)
            .is_some()
    );

    assert!(tracker.admit_gap_attempt(revised, 11).is_some());
    assert_eq!(tracker.attempt_gap_id, Some(revised));
    assert_eq!(tracker.batches_processed, 0);
    assert_eq!(tracker.live_edge_batches_processed, 0);
}

#[test]
fn gap_repair_budget_is_scoped_to_gap_ordinal() {
    let mut tracker = TimelineGapRepairTracker::default();
    let first = projected_gap_id(7, 1);
    let another = projected_gap_id(7, 2);

    assert!(tracker.admit_gap_attempt(first, 11).is_some());
    assert!(
        tracker
            .record_batch(TimelineGapRepairTrigger::Automatic)
            .is_some()
    );

    assert!(tracker.admit_gap_attempt(another, 11).is_some());
    assert_eq!(tracker.attempt_gap_id, Some(another));
    assert_eq!(tracker.batches_processed, 0);
}

#[test]
fn gap_repair_budget_is_scoped_to_explicit_demand_revision() {
    let mut tracker = TimelineGapRepairTracker::default();
    let id = projected_gap_id(7, 1);

    assert!(tracker.admit_gap_attempt(id, 11).is_some());
    assert!(
        tracker
            .record_batch(TimelineGapRepairTrigger::Automatic)
            .is_some()
    );

    assert!(tracker.admit_gap_attempt(id, 12).is_some());
    assert_eq!(tracker.attempt_gap_id, Some(id));
    assert_eq!(tracker.batches_processed, 0);
}

#[test]
fn gap_repair_budget_is_scoped_to_room_reselection_demand() {
    let projected = vec![(0, projected_gap_position(7, 1, 8))];
    let id = projected_gap_id(7, 1);
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.replace_projected_gaps(projected, Some((5, 10)), &[id]);
    let initial_demand = tracker.begin_explicit_demand();
    assert!(tracker.admit_gap_attempt(id, initial_demand).is_some());
    assert_eq!(
        tracker.record_batch(TimelineGapRepairTrigger::Automatic),
        Some(1)
    );

    let reselection_demand = tracker.begin_explicit_demand();

    assert_ne!(initial_demand, reselection_demand);
    assert_eq!(tracker.batches_processed, 1);
    assert!(matches!(
        tracker.evaluate_viewport_wake(Some((5, 10)), &[id]),
        GapRepairViewportWakeDecision::Wake { .. }
    ));
    assert!(tracker.admit_gap_attempt(id, reselection_demand).is_some());
    assert_eq!(tracker.batches_processed, 0);
}

#[test]
fn gap_repair_budget_is_scoped_to_newly_visible_demand() {
    let projected = vec![(0, projected_gap_position(7, 1, 8))];
    let id = projected_gap_id(7, 1);
    let mut tracker = TimelineGapRepairTracker::default();
    let initial_demand = tracker.begin_explicit_demand();
    tracker.replace_projected_gaps(projected, None, &[]);
    assert!(tracker.admit_gap_attempt(id, initial_demand).is_some());
    assert_eq!(
        tracker.record_batch(TimelineGapRepairTrigger::Automatic),
        Some(1)
    );

    assert!(matches!(
        tracker.evaluate_viewport_wake(None, &[id]),
        GapRepairViewportWakeDecision::Wake {
            candidate: ProjectedGapCandidate {
                relation: ProjectedGapRelation::ExplicitVisible,
                ..
            }
        }
    ));
    let visible_demand = tracker.demand_revision;

    assert_ne!(initial_demand, visible_demand);
    assert!(tracker.admit_gap_attempt(id, visible_demand).is_some());
    assert_eq!(tracker.batches_processed, 0);
    assert!(matches!(
        tracker.evaluate_viewport_wake(None, &[id]),
        GapRepairViewportWakeDecision::IdleUnchangedCandidate { .. }
    ));
    assert_eq!(tracker.demand_revision, visible_demand);
}

#[test]
fn repair_projection_waits_for_the_exact_tagged_batch() {
    let mut correlation = TimelineGapProjectionCorrelation::default();
    let operation = historical_causal_projection_operation(11);
    correlation.begin(9, operation);

    // An unrelated live diff can consume batch 5 without satisfying the repair.
    assert_eq!(
        correlation.complete(9, operation, Some(1)),
        TimelineGapProjectionCompletion::Pending
    );
    assert_eq!(
        correlation.observe(
            CausalProjectionId {
                actor_generation: 9,
                operation: historical_causal_projection_operation(10),
                projection_batch: 1,
            },
            TimelineBatchId(5),
        ),
        None
    );
    assert_eq!(
        correlation.observe(
            CausalProjectionId {
                actor_generation: 9,
                operation,
                projection_batch: 1,
            },
            TimelineBatchId(6),
        ),
        Some(TimelineBatchId(6))
    );
}

#[test]
fn repair_projection_uses_the_last_sdk_projection_batch() {
    let mut correlation = TimelineGapProjectionCorrelation::default();
    let operation = historical_causal_projection_operation(7);
    correlation.begin(4, operation);
    assert_eq!(
        correlation.observe(
            CausalProjectionId {
                actor_generation: 4,
                operation,
                projection_batch: 1,
            },
            TimelineBatchId(20),
        ),
        None
    );
    assert_eq!(
        correlation.complete(4, operation, Some(2)),
        TimelineGapProjectionCompletion::Pending
    );
    assert_eq!(
        correlation.observe(
            CausalProjectionId {
                actor_generation: 4,
                operation,
                projection_batch: 2,
            },
            TimelineBatchId(21),
        ),
        Some(TimelineBatchId(21))
    );
}

#[test]
fn gap_only_cache_reveal_requires_no_render_fence() {
    let mut correlation = TimelineGapProjectionCorrelation::default();
    let operation = historical_causal_projection_operation(3);
    correlation.begin(2, operation);
    assert_eq!(
        correlation.complete(2, operation, None),
        TimelineGapProjectionCompletion::NoDiff
    );
    assert!(!correlation.is_pending());
}

#[tokio::test]
async fn gap_repair_room_switch_cancels_completion() {
    use matrix_sdk::{
        linked_chunk::{ChunkIdentifier, LinkedChunkId, Position, Update},
        test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate},
    };
    use matrix_sdk_base::event_cache::Gap;
    use matrix_sdk_test::{ALICE, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = matrix_sdk::ruma::room_id!("!cancel-gap:example.org");
    let older_id = matrix_sdk::ruma::event_id!("$cancel-older:example.org");
    let newer_id = matrix_sdk::ruma::event_id!("$cancel-newer:example.org");
    let missing_id = matrix_sdk::ruma::event_id!("$cancel-missing:example.org");
    let factory = EventFactory::new().room(room_id).sender(&ALICE);
    {
        let store = client
            .event_cache_store()
            .lock()
            .await
            .expect("cache store");
        store
            .as_clean()
            .expect("clean cache store")
            .handle_linked_chunk_updates(
                LinkedChunkId::Room(room_id),
                vec![
                    Update::NewItemsChunk {
                        previous: None,
                        new: ChunkIdentifier::new(0),
                        next: None,
                    },
                    Update::PushItems {
                        at: Position::new(ChunkIdentifier::new(0), 0),
                        items: vec![factory.text_msg("older").event_id(older_id).into_event()],
                    },
                    Update::NewGapChunk {
                        previous: Some(ChunkIdentifier::new(0)),
                        new: ChunkIdentifier::new(1),
                        next: None,
                        gap: Gap {
                            token: "cancel-gap-token".to_owned(),
                        },
                    },
                    Update::NewItemsChunk {
                        previous: Some(ChunkIdentifier::new(1)),
                        new: ChunkIdentifier::new(2),
                        next: None,
                    },
                    Update::PushItems {
                        at: Position::new(ChunkIdentifier::new(2), 0),
                        items: vec![factory.text_msg("newer").event_id(newer_id).into_event()],
                    },
                ],
            )
            .await
            .expect("seed persisted gap");
    }
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscribe");
    server.sync_joined_room(&client, room_id).await;
    server
        .mock_room_messages()
        .match_from("cancel-gap-token")
        .match_limit(64)
        .ok(RoomMessagesResponseTemplate::default().events(vec![
            factory.text_msg("missing").event_id(missing_id),
            factory.text_msg("older").event_id(older_id),
        ]))
        .expect(1)
        .named("old-actor-real-gap-repair")
        .mount()
        .await;

    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(
        AccountKey("@cancel-gap:example.org".to_owned()),
        room_id.to_string(),
    );
    let projection_request_id = fake_rid(27_500);
    let (action_tx, mut action_rx) = mpsc::channel(128);
    let (event_tx, mut event_rx) = broadcast::channel(128);
    let (manager_tx, _manager_rx) = mpsc::channel(16);
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    manager.action_tx = action_tx;
    manager.event_tx = event_tx;
    manager.msg_tx = manager_tx;
    manager.test_session_available = false;
    manager
        .handle_subscribe(
            projection_request_id,
            key.clone(),
            true,
            true,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;

    let old_actor_generation = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                key: emitted_key,
                actor_generation,
                generation,
                ..
            }) = event_rx.recv().await.expect("initial actor event")
                && request_id == projection_request_id
                && emitted_key == key
            {
                let _ = generation;
                break actor_generation;
            }
        }
    })
    .await
    .expect("real actor initial projection");
    let old_actor_tx = manager
        .timelines
        .get(&key)
        .expect("old room actor")
        .tx
        .clone();

    let (reached_tx, reached_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let (forwarded_tx, forwarded_rx) = oneshot::channel();
    let (armed_tx, armed_rx) = oneshot::channel();
    assert!(
        manager
            .timelines
            .get(&key)
            .expect("old room actor")
            .send(TimelineActorMessage::TestArmGapRepairCompletionPause {
                pause: TestGapRepairCompletionPause {
                    reached: reached_tx,
                    release: release_rx,
                    forwarded: forwarded_tx,
                },
                acknowledged: armed_tx,
            })
            .await
    );
    armed_rx.await.expect("completion pause armed");
    assert!(
        manager
            .timelines
            .get(&key)
            .expect("old room actor")
            .send(TimelineActorMessage::InspectTimelineGaps {
                trigger: TimelineGapRepairTrigger::Manual,
            })
            .await
    );

    let started_generation = tokio::time::timeout(Duration::from_secs(5), async {
        'started: loop {
            for action in action_rx.recv().await.expect("gap repair action channel") {
                if let AppAction::TimelineGapRepairStarted {
                    room_id: started_room_id,
                    generation,
                    ..
                } = action
                    && started_room_id == room_id.as_str()
                {
                    break 'started generation;
                }
            }
        }
    })
    .await
    .expect("real SDK gap repair started");
    tokio::time::timeout(Duration::from_secs(5), reached_rx)
        .await
        .expect("real SDK repair reached the session-to-actor completion boundary")
        .expect("completion pause sender");

    let (old_barrier_tx, old_barrier_rx) = oneshot::channel();
    assert!(
        manager
            .timelines
            .get(&key)
            .expect("old room actor")
            .send(TimelineActorMessage::Barrier(old_barrier_tx))
            .await
    );
    old_barrier_rx.await.expect("old actor pre-switch barrier");
    while action_rx.try_recv().is_ok() {}
    while event_rx.try_recv().is_ok() {}

    manager
        .handle_command(TimelineCommand::Unsubscribe {
            request_id: fake_rid(27_501),
            key: key.clone(),
        })
        .await;
    assert!(!manager.timelines.contains_key(&key));
    let replacement_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    assert_ne!(old_actor_generation, replacement_generation);
    while action_rx.try_recv().is_ok() {}
    while event_rx.try_recv().is_ok() {}

    let _ = release_tx.send(());
    let completion_forwarded = match tokio::time::timeout(Duration::from_secs(1), forwarded_rx)
        .await
        .expect("paused repair worker must settle after old actor drop")
    {
        Ok(forwarded) => forwarded,
        Err(_) => false,
    };
    let old_actor_closed = tokio::time::timeout(Duration::from_millis(100), old_actor_tx.closed())
        .await
        .is_ok();
    if !old_actor_closed {
        let (barrier_tx, barrier_rx) = oneshot::channel();
        if old_actor_tx
            .send(TimelineActorMessage::Barrier(barrier_tx))
            .await
            .is_ok()
        {
            let _ = tokio::time::timeout(Duration::from_secs(1), barrier_rx).await;
        }
    }

    let mut stale_actions = Vec::new();
    while let Ok(actions) = action_rx.try_recv() {
        for action in actions {
            let label = match action {
                AppAction::TimelineGapRepairProgressed { .. } => Some("Progressed"),
                AppAction::TimelineGapRepairFailed { .. } => Some("Failed"),
                AppAction::TimelineContinuityInspectionStarted { .. } => {
                    Some("inspection continuation")
                }
                _ => None,
            };
            if let Some(label) = label {
                stale_actions.push(label);
            }
        }
    }
    let mut stale_core_event_count = 0;
    while event_rx.try_recv().is_ok() {
        stale_core_event_count += 1;
    }

    assert!(
        !completion_forwarded,
        "the released old-generation repair completion reached its actor mailbox"
    );
    assert!(
        old_actor_closed,
        "the unsubscribed actor channel stayed open"
    );
    assert!(
        stale_actions.is_empty(),
        "old generation {old_actor_generation} published reducer work after replacement generation {replacement_generation}: {stale_actions:?}; repair generation {started_generation}"
    );
    assert_eq!(
        stale_core_event_count, 0,
        "old generation {old_actor_generation} published CoreEvent output after replacement generation {replacement_generation}"
    );
}
