#![recursion_limit = "512"]

use std::sync::Arc;

use koushi_core::room_subscription_residency_test_support::RoomSubscriptionResidencyHarness;
use koushi_core::{AccountKey, TimelineKey, TimelineKind};
use koushi_sdk::MatrixClientSession;
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::ruma::room_id;
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_ui::room_list_service::RoomListService;

async fn harness() -> (MatrixMockServer, RoomSubscriptionResidencyHarness) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@resident:example.invalid".to_owned(),
            device_id: "RESIDENT".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let service = Arc::new(
        RoomListService::new(client)
            .await
            .expect("room list service"),
    );
    (
        server,
        RoomSubscriptionResidencyHarness::with_room_list_service(session, service).await,
    )
}

fn room_key(room_id: &str) -> TimelineKey {
    TimelineKey::room(AccountKey("@resident:example.invalid".to_owned()), room_id)
}

fn thread_key(room_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@resident:example.invalid".to_owned()),
        kind: TimelineKind::Thread {
            room_id: room_id.to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
        },
    }
}

fn focused_key(room_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@resident:example.invalid".to_owned()),
        kind: TimelineKind::Focused {
            room_id: room_id.to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    }
}

#[tokio::test]
async fn room_subscription_residency_final_actor_unsubscribe_retains_room() {
    let (_server, mut harness) = harness().await;
    let room_id = room_id!("!resident-a:example.invalid").to_string();

    harness.admit_timeline_key(room_key(&room_id)).await;
    harness.admit_timeline_key(thread_key(&room_id)).await;
    harness.unsubscribe(room_key(&room_id)).await;
    harness.unsubscribe(thread_key(&room_id)).await;

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.actor_count, 0);
    assert_eq!(snapshot.lease_count, 0);
    assert_eq!(snapshot.desired_rooms, vec![room_id.clone()]);
    assert_eq!(snapshot.active_rooms, vec![room_id.clone()]);

    let extra_room_id = room_id!("!resident-extra:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&extra_room_id)).await;

    let snapshot = harness.snapshot();
    let mut expected_rooms = vec![room_id, extra_room_id];
    expected_rooms.sort();
    assert_eq!(snapshot.desired_rooms, expected_rooms);
    assert_eq!(snapshot.active_rooms, expected_rooms);
}

#[tokio::test]
async fn room_subscription_residency_actor_build_failure_retains_admitted_room() {
    let (_server, mut harness) = harness().await;
    let room_id = room_id!("!resident-build-failure:example.invalid").to_string();

    harness.admit_build_failure(&room_id).await;

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.actor_count, 0);
    assert_eq!(snapshot.lease_count, 0);
    assert_eq!(snapshot.desired_rooms, vec![room_id.clone()]);
    assert_eq!(snapshot.active_rooms, vec![room_id]);
}

#[tokio::test]
async fn room_subscription_residency_has_no_count_or_lru_eviction() {
    let (_server, mut harness) = harness().await;
    let mut room_ids = Vec::new();
    let mut keys = Vec::new();
    for index in 0..140 {
        let room_id = format!("!resident-{index:03}:example.invalid");
        room_ids.push(room_id.clone());
        let key = room_key(&room_id);
        keys.push(key.clone());
        harness.admit_timeline_key(key).await;
    }
    for key in keys {
        harness.unsubscribe(key).await;
    }

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.actor_count, 0);
    assert_eq!(snapshot.lease_count, 0);
    assert_eq!(snapshot.desired_rooms, room_ids);
    assert_eq!(snapshot.active_rooms, snapshot.desired_rooms);

    let extra_room_id = room_id!("!resident-140:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&extra_room_id)).await;

    let snapshot = harness.snapshot();
    let mut expected_rooms = room_ids;
    expected_rooms.push(extra_room_id);
    expected_rooms.sort();
    assert_eq!(snapshot.desired_rooms, expected_rooms);
    assert_eq!(snapshot.active_rooms, expected_rooms);
}

#[tokio::test]
async fn room_subscription_residency_room_thread_focused_share_one_room() {
    let (_server, mut harness) = harness().await;
    let room_id = room_id!("!resident-shared:example.invalid").to_string();

    harness.admit_timeline_key(room_key(&room_id)).await;
    let first = harness.snapshot();
    harness.admit_timeline_key(thread_key(&room_id)).await;
    harness.admit_timeline_key(focused_key(&room_id)).await;
    let after_admission = harness.snapshot();
    assert_eq!(after_admission.actor_count, 3);
    assert_eq!(after_admission.lease_count, 3);
    assert_eq!(after_admission.desired_rooms, vec![room_id.clone()]);
    assert_eq!(after_admission.active_rooms, vec![room_id.clone()]);
    assert_eq!(after_admission.sdk_generation, first.sdk_generation);

    harness.unsubscribe(room_key(&room_id)).await;
    harness.unsubscribe(thread_key(&room_id)).await;
    harness.unsubscribe(focused_key(&room_id)).await;

    let final_snapshot = harness.snapshot();
    assert_eq!(final_snapshot.actor_count, 0);
    assert_eq!(final_snapshot.lease_count, 0);
    assert_eq!(final_snapshot.desired_rooms, vec![room_id.clone()]);
    assert_eq!(final_snapshot.active_rooms, vec![room_id.clone()]);

    let extra_room_id = room_id!("!resident-extra:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&extra_room_id)).await;

    let final_snapshot = harness.snapshot();
    let mut expected_rooms = vec![room_id, extra_room_id];
    expected_rooms.sort();
    assert_eq!(final_snapshot.desired_rooms, expected_rooms);
    assert_eq!(final_snapshot.active_rooms, expected_rooms);
}

#[tokio::test]
async fn room_subscription_residency_opened_visible_restored_are_unioned() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-union-a:example.invalid").to_string();
    let room_b = room_id!("!resident-union-b:example.invalid").to_string();
    let room_c = room_id!("!resident-union-c:example.invalid").to_string();
    let room_d = room_id!("!resident-union-d:example.invalid").to_string();

    harness.sync_started(7).await;
    harness.offer_restore(7, &[&room_a, &room_b], true).await;
    harness.observe_visible(7, &[&room_b, &room_c]).await;
    harness.admit_timeline_key(room_key(&room_d)).await;

    let snapshot = harness.snapshot();
    assert_eq!(
        snapshot.desired_rooms,
        vec![room_a, room_b, room_c, room_d],
        "opened, visible, and continuity-proven restored rooms must be additive"
    );
}

#[tokio::test]
async fn room_subscription_residency_identical_visible_range_is_noop() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-visible-a:example.invalid").to_string();

    harness.sync_started(8).await;
    harness.observe_visible(8, &[&room_a]).await;
    let first = harness.snapshot();
    harness.observe_visible(8, &[&room_a]).await;
    let second = harness.snapshot();

    assert_eq!(second.desired_rooms, vec![room_a]);
    assert_eq!(second.sdk_generation, first.sdk_generation);
}

#[tokio::test]
async fn room_subscription_residency_invalid_or_stale_visible_is_rejected() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-visible-valid:example.invalid").to_string();
    let room_b = room_id!("!resident-visible-stale:example.invalid").to_string();
    let room_c = room_id!("!resident-visible-fresh:example.invalid").to_string();

    harness.sync_started(9).await;
    harness.observe_visible(9, &[&room_a]).await;
    harness
        .observe_visible_entries(9, &[(&room_c, true), (&room_c, true)])
        .await;
    harness
        .observe_visible_entries(9, &[("not-a-room-id", false), (&room_a, true)])
        .await;
    harness.observe_visible(8, &[&room_b]).await;

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.desired_rooms, vec![room_a]);
}

#[tokio::test]
async fn room_subscription_residency_unproven_restore_is_rejected() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-restore-unproven:example.invalid").to_string();
    let room_b = room_id!("!resident-restore-proven:example.invalid").to_string();

    harness.sync_started(10).await;
    harness.seed_sdk_subscriptions(&[&room_a]).await;
    harness.offer_restore(10, &[&room_a], false).await;
    assert!(harness.snapshot().desired_rooms.is_empty());

    harness.offer_restore(10, &[&room_b], true).await;
    assert_eq!(harness.snapshot().desired_rooms, vec![room_b]);
}

#[tokio::test]
async fn room_subscription_residency_unknown_pos_reconciles_complete_intent() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-expiry-a:example.invalid").to_string();
    let room_b = room_id!("!resident-expiry-b:example.invalid").to_string();

    harness.sync_started(11).await;
    harness.admit_timeline_key(room_key(&room_a)).await;
    harness.admit_timeline_key(room_key(&room_b)).await;
    harness.expire_sdk_subscriptions().await;
    harness.observe_visible(11, &[&room_a]).await;

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.desired_rooms, vec![room_a, room_b]);
    assert_eq!(snapshot.active_rooms, snapshot.desired_rooms);
}

#[tokio::test]
async fn room_subscription_residency_binding_cleared_leave_is_rejected_without_tombstone() {
    let (_server, mut harness) = harness().await;
    let room_id = room_id!("!resident-binding-cleared:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&room_id)).await;
    harness.clear_residency_binding();

    assert!(!harness.leave_room(&room_id, true).await);

    let snapshot = harness.snapshot();
    assert!(
        snapshot.tombstoned_rooms.is_empty(),
        "binding-missing rejection must not tombstone the room"
    );
    assert_eq!(snapshot.desired_rooms, vec![room_id.clone()]);
}

#[tokio::test]
async fn room_subscription_residency_leave_and_decline_share_success_terminal() {
    let (_server, mut leave_harness) = harness().await;
    let room_a = room_id!("!resident-leave-a:example.invalid").to_string();
    let room_b = room_id!("!resident-leave-b:example.invalid").to_string();
    leave_harness.admit_timeline_key(room_key(&room_a)).await;
    leave_harness.admit_timeline_key(room_key(&room_b)).await;
    assert!(leave_harness.leave_room(&room_a, true).await);
    assert_eq!(leave_harness.snapshot().desired_rooms, vec![room_b.clone()]);

    let (_server, mut decline_harness) = harness().await;
    decline_harness.admit_timeline_key(room_key(&room_a)).await;
    decline_harness.admit_timeline_key(room_key(&room_b)).await;
    assert!(decline_harness.decline_invite(&room_a, true).await);
    assert_eq!(
        decline_harness.snapshot().desired_rooms,
        vec![room_b.clone()]
    );

    let (_server, mut failed_harness) = harness().await;
    failed_harness.admit_timeline_key(room_key(&room_a)).await;
    failed_harness.admit_timeline_key(room_key(&room_b)).await;
    assert!(!failed_harness.leave_room(&room_a, false).await);
    assert!(!failed_harness.decline_invite(&room_a, false).await);
    assert_eq!(
        failed_harness.snapshot().desired_rooms,
        vec![room_a, room_b]
    );
}

#[tokio::test]
async fn room_subscription_residency_pre_sync_leave_targets_replacement_manager() {
    let (_server, mut harness) = harness().await;
    let probe = harness.pre_sync_mismatch_probe().await;
    assert_eq!(probe.room_session.as_deref(), Some("A"));
    assert_eq!(probe.bound_session.as_deref(), Some("B"));
    assert!(!probe.pointer_equal);
    assert!(probe.mismatch_probe);
}

#[tokio::test]
async fn room_subscription_residency_lost_leave_ack_fails_closed() {
    let (_server, mut harness) = harness().await;
    let probe = harness.lost_leave_acknowledgement().await;
    assert_eq!(probe.operation_failed_sdk_count, 1);
    assert_eq!(probe.room_left_count, 0);
    assert_eq!(probe.success_action_count, 0);
    assert_eq!(probe.ack_diagnostic_count, 1);
}

#[tokio::test]
async fn room_subscription_residency_pre_sync_leave_blocks_restore_resurrection() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-pre-sync-left:example.invalid").to_string();
    let room_b = room_id!("!resident-pre-sync-kept:example.invalid").to_string();

    harness.leave_room(&room_a, true).await;
    harness.sync_started(12).await;
    harness.offer_restore(12, &[&room_a, &room_b], true).await;

    assert_eq!(harness.snapshot().desired_rooms, vec![room_b]);
}

#[tokio::test]
async fn room_subscription_residency_inflight_leave_drains_before_replacement() {
    let (_server, mut harness) = harness().await;
    let probe = harness.inflight_leave_replacement().await;
    assert!(probe.old_manager_alive);
    assert!(probe.acknowledgement_before_replacement);
    assert!(probe.settlement_before_replacement);
    assert!(probe.replacement_completed);
    assert!(!probe.late_terminal_after_replacement);
}

#[tokio::test]
async fn room_subscription_residency_delayed_projection_cannot_clear_leave() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-delayed-left:example.invalid").to_string();
    harness.sync_started(13).await;
    harness.leave_room(&room_a, true).await;
    harness.observe_visible(13, &[&room_a]).await;
    assert_eq!(harness.snapshot().desired_rooms, Vec::<String>::new());
    assert_eq!(harness.snapshot().tombstoned_rooms, vec![room_a]);
}

#[tokio::test]
async fn room_subscription_residency_rejoin_requires_ordered_transition() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-ordered-rejoin:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&room_a)).await;
    harness.leave_room(&room_a, true).await;
    harness.sync_started(14).await;
    harness
        .membership_sequence(14, &[(&room_a, "joined"), (&room_a, "left")])
        .await;
    assert!(harness.snapshot().desired_rooms.is_empty());
    harness
        .membership_sequence(14, &[(&room_a, "left"), (&room_a, "joined")])
        .await;
    assert_eq!(harness.snapshot().desired_rooms, vec![room_a]);
}

#[tokio::test]
async fn room_subscription_residency_stale_membership_cannot_clear_leave() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-stale-membership:example.invalid").to_string();
    harness.leave_room(&room_a, true).await;
    harness
        .stale_membership_sequence(
            13,
            &[(&room_a, "left"), (&room_a, "joined"), (&room_a, "invited")],
        )
        .await;
    assert!(harness.snapshot().desired_rooms.is_empty());
    assert_eq!(harness.snapshot().tombstoned_rooms, vec![room_a]);
}

#[tokio::test]
async fn room_subscription_residency_local_rejoin_is_replacement_fenced() {
    let (_server, mut harness) = harness().await;
    let probe = harness.local_rejoin_replacement_fence().await;
    assert!(probe.acknowledgement_before_replacement);
    assert!(probe.settlement_before_replacement);
    assert!(probe.replacement_completed);
    assert!(!probe.late_terminal_after_replacement);
}

#[tokio::test]
async fn room_subscription_residency_failed_operations_settle_before_replacement() {
    let (_server, mut harness) = harness().await;
    let probe = harness.failed_operations_before_replacement().await;
    assert!(probe.settlement_before_replacement);
    assert!(probe.replacement_completed);
    assert!(!probe.late_terminal_after_replacement);
}

#[tokio::test]
async fn room_subscription_residency_final_permit_drop_cannot_miss_drain() {
    let (_server, mut harness) = harness().await;
    let probe = harness.final_permit_drop_probe().await;
    assert!(!probe.accepting_after_close);
    assert_eq!(probe.active_count_after_close, 0);
    assert!(probe.new_admission_rejected);
    assert!(probe.drain_completed);
}

#[tokio::test]
async fn room_subscription_residency_timeline_setup_precedes_room_observation() {
    let (_server, mut harness) = harness().await;
    assert!(harness.timeline_setup_precedes_room_observation().await);
}

#[tokio::test]
async fn room_subscription_residency_manager_teardown_is_account_isolated() {
    let (_server, mut harness) = harness().await;
    let probe = harness.account_teardown_probe().await;
    assert!(probe.binding_cleared);
    assert!(probe.post_clear_admission_rejected);
    assert!(probe.post_clear_failure_is_sdk);
    assert_eq!(probe.operation_control_reached_count, 1);
    assert!(probe.shutdown_incomplete_while_gap_held);
    assert!(probe.shutdown_incomplete_while_permit_held);
    assert!(probe.acknowledgement_before_shutdown);
    assert!(probe.settlement_before_shutdown);
    assert!(probe.shutdown_completed);
    assert_eq!(probe.matching_terminal_count, 1);
    assert!(probe.no_late_terminal);

    let room_a = room_id!("!resident-account-a:example.invalid").to_string();
    let room_b = room_id!("!resident-account-b:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&room_a)).await;
    harness.replace_account_and_restore(&[&room_b]).await;
    assert_eq!(harness.snapshot().desired_rooms, vec![room_b]);
}

#[tokio::test]
async fn room_subscription_residency_rapid_intents_serialize() {
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-rapid-a:example.invalid").to_string();
    let room_b = room_id!("!resident-rapid-b:example.invalid").to_string();
    harness.sync_started(15).await;
    harness.observe_visible(15, &[&room_a, &room_b]).await;
    harness.admit_timeline_key(room_key(&room_a)).await;
    assert_eq!(harness.snapshot().desired_rooms, vec![room_a, room_b]);
}

#[tokio::test]
async fn room_subscription_residency_diagnostics_are_private_safe_and_closed() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let diagnostic_start = koushi_diagnostics::test_support::detail_cursor();
    let (_server, mut harness) = harness().await;
    let room_a = room_id!("!resident-diagnostic:example.invalid").to_string();
    harness.admit_timeline_key(room_key(&room_a)).await;
    harness.leave_room(&room_a, true).await;
    harness.decline_invite(&room_a, true).await;

    let records = koushi_diagnostics::test_support::detail_records_since(diagnostic_start)
        .into_iter()
        .filter(|record| record.event.source == "core.subscription")
        .collect::<Vec<_>>();
    assert!(
        !records.is_empty(),
        "residency operations must append subscription diagnostics"
    );
    let approved_sources = [
        "opened",
        "visible_range",
        "restore",
        "room_left",
        "room_rejoined",
        "membership",
        "session_restart",
    ];
    let source_tokens = records
        .iter()
        .flat_map(|record| record.event.fields.iter())
        .filter(|field| matches!(field.key, "source" | "trigger"))
        .filter_map(|field| match &field.value {
            koushi_diagnostics::DiagnosticValue::Token(token) => Some(*token),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!source_tokens.is_empty());
    assert!(
        source_tokens
            .iter()
            .all(|token| approved_sources.contains(token))
    );

    let reconcile = records
        .iter()
        .find(|record| record.event.stage == "reconcile")
        .expect("residency reconcile diagnostic");
    let keys = reconcile
        .event
        .fields
        .iter()
        .map(|field| field.key)
        .collect::<std::collections::BTreeSet<_>>();
    for key in [
        "previous_bucket",
        "desired_bucket",
        "added_bucket",
        "removed_bucket",
        "retained_bucket",
        "generation_before",
        "generation_after",
    ] {
        assert!(
            keys.contains(key),
            "missing reconcile diagnostic key: {key}"
        );
    }

    for record in records {
        let text = format!("{:?}", record.event);
        assert!(!text.contains("!resident-diagnostic"));
        assert!(!text.contains("@"));
    }
}
