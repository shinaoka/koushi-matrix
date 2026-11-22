use std::collections::HashSet;

use super::{
    READ_STATE_OUTBOX_ENTRY_LIMIT, READ_STATE_WAITER_LIMIT, ReadAdmissionRejection,
    ReadAdmissionStatus, ReadCompletionDisposition, ReadNetworkFailure, ReadNetworkOutcome,
    ReadOperationFence, ReadPositionEvidence, ReadStateEngine, ReadStateFailureKind, ReadStateKey,
    ReadTarget, ReadWaiterId, ReadWaiterTerminal, ReadWakeResult, confirmed_boundary_accepts,
};

const SESSION: u64 = 7;

fn public(room: &str) -> ReadStateKey {
    ReadStateKey::PublicUnthreaded {
        room_id: room.to_owned(),
    }
}

fn thread(room: &str, root: &str) -> ReadStateKey {
    ReadStateKey::ThreadRead {
        room_id: room.to_owned(),
        root_event_id: root.to_owned(),
    }
}

fn fully_read(room: &str) -> ReadStateKey {
    ReadStateKey::FullyReadAndPrivateUnthreaded {
        room_id: room.to_owned(),
    }
}

fn unordered(event: &str) -> ReadTarget {
    ReadTarget::new(event.to_owned())
}

fn positioned(event: &str, generation: u64, rank: u64) -> ReadTarget {
    ReadTarget::with_position(
        event.to_owned(),
        ReadPositionEvidence {
            generation: generation.into(),
            rank,
        },
    )
}

fn waiter(value: u64) -> ReadWaiterId {
    ReadWaiterId::new(value)
}

fn failed() -> ReadNetworkOutcome {
    ReadNetworkOutcome::Failed(ReadNetworkFailure::new(ReadStateFailureKind::Sdk))
}

#[test]
fn read_state_keys_keep_public_thread_and_fully_read_bundles_distinct() {
    let room = "synthetic-room";
    let keys = HashSet::from([
        public(room),
        thread(room, "synthetic-root-a"),
        thread(room, "synthetic-root-b"),
        fully_read(room),
    ]);

    assert_eq!(keys.len(), 4);
}

#[test]
fn position_evidence_coalesces_to_the_newest_candidate_and_keeps_waiters() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);

    let first = engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-10", 3, 10),
        waiter(1),
    );
    let newer = engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-12", 3, 12),
        waiter(2),
    );
    let older = engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-11", 3, 11),
        waiter(3),
    );

    assert_eq!(first.status(), ReadAdmissionStatus::Accepted);
    assert_eq!(engine.session_generation(), SESSION);
    assert_eq!(newer.status(), ReadAdmissionStatus::Accepted);
    assert_eq!(older.status(), ReadAdmissionStatus::Coalesced);
    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), 3);
    assert!(engine.has_candidate(&key, "synthetic-event-12"));
}

#[test]
fn unordered_latest_admission_is_the_only_desired_target() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-a"),
        waiter(1),
    );
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-b"),
        waiter(2),
    );

    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), 2);
    assert!(!engine.has_candidate(&key, "synthetic-event-a"));
    assert!(engine.has_candidate(&key, "synthetic-event-b"));
}

#[test]
fn new_visible_waiter_overtakes_restored_background_candidates() {
    let key = public("synthetic-room");
    let mut seed = ReadStateEngine::new(SESSION);
    seed.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-restored-a"),
        waiter(1),
    );
    seed.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-restored-b"),
        waiter(2),
    );
    let mut engine = ReadStateEngine::restore(SESSION, seed.persistence_snapshot())
        .expect("restore valid background candidates");
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-visible"),
        waiter(3),
    );

    let ReadWakeResult::Start(operation) = engine.wake(&key) else {
        panic!("visible waiter must start");
    };
    assert_eq!(operation.target().event_id(), "synthetic-visible");
}

#[test]
fn failed_unordered_background_target_is_retained_for_retry() {
    let key = public("synthetic-room");
    let mut seed = ReadStateEngine::new(SESSION);
    seed.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-background-a"),
        waiter(1),
    );
    seed.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-background-b"),
        waiter(2),
    );
    let mut engine = ReadStateEngine::restore(SESSION, seed.persistence_snapshot())
        .expect("restore valid background target");

    let ReadWakeResult::Start(first) = engine.wake(&key) else {
        panic!("background target must start");
    };
    assert_eq!(first.target().event_id(), "synthetic-background-b");
    engine.complete(&key, first.fence(), failed());

    assert_eq!(engine.candidate_count(&key), 1);
    assert!(engine.has_candidate(&key, "synthetic-background-b"));
}

#[test]
fn candidates_from_different_position_generations_are_not_ordered() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-a", 4, 100),
        waiter(1),
    );
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-b", 5, 1),
        waiter(2),
    );

    assert_eq!(engine.candidate_count(&key), 1);
    assert!(engine.has_candidate(&key, "synthetic-event-b"));
}

#[test]
fn key_limit_rejects_the_129th_key_without_eviction() {
    let mut engine = ReadStateEngine::new(SESSION);
    for index in 0..READ_STATE_OUTBOX_ENTRY_LIMIT {
        let key = public(&format!("synthetic-room-{index}"));
        let result = engine.admit(
            SESSION,
            key,
            unordered("synthetic-event"),
            waiter(index as u64),
        );
        assert_ne!(
            result.status(),
            ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity)
        );
    }

    let rejected = engine.admit(
        SESSION,
        public("synthetic-room-over-capacity"),
        unordered("synthetic-event-over-capacity"),
        waiter(1000),
    );

    assert_eq!(
        rejected.status(),
        ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity)
    );
    assert_eq!(
        engine.persistence_snapshot().entry_count(),
        READ_STATE_OUTBOX_ENTRY_LIMIT
    );
}

#[test]
fn waiter_limit_rejects_the_thirty_third_request_without_eviction() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    for index in 0..READ_STATE_WAITER_LIMIT {
        let result = engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event"),
            waiter(index as u64),
        );
        assert_ne!(
            result.status(),
            ReadAdmissionStatus::Rejected(ReadAdmissionRejection::WaiterCapacity)
        );
    }

    let rejected = engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event"),
        waiter(100),
    );

    assert_eq!(
        rejected.status(),
        ReadAdmissionStatus::Rejected(ReadAdmissionRejection::WaiterCapacity)
    );
    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), READ_STATE_WAITER_LIMIT);
}

#[test]
fn newer_candidate_supersedes_active_and_stale_completion_cannot_regress() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-old", 8, 10),
        waiter(1),
    );
    let old_operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };
    assert_eq!(old_operation.key(), &key);
    assert_eq!(old_operation.target().event_id(), "synthetic-event-old");
    assert_eq!(
        old_operation.target().position(),
        Some(ReadPositionEvidence {
            generation: 8,
            rank: 10,
        })
    );
    assert_eq!(old_operation.fence().session_generation(), SESSION);

    let admission = engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-new", 8, 20),
        waiter(2),
    );

    assert_eq!(
        admission.superseded_operation(),
        Some(old_operation.fence())
    );
    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), 2);
    assert!(engine.has_candidate(&key, "synthetic-event-new"));
    assert_eq!(engine.active_operation(&key), Some(old_operation.fence()));

    let stale = engine.complete(&key, old_operation.fence(), ReadNetworkOutcome::Succeeded);
    assert_eq!(
        stale.disposition(),
        ReadCompletionDisposition::StaleDiscarded
    );
    assert!(stale.settlements().is_empty());
    assert!(engine.has_candidate(&key, "synthetic-event-new"));
    assert_eq!(engine.waiter_count(&key), 2);

    let new_operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a replacement start, got {other:?}"),
    };
    assert!(
        new_operation.fence().operation_generation() > old_operation.fence().operation_generation()
    );
}

#[test]
fn timeout_and_failure_settle_waiters_but_retain_desired_for_retry() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event"),
        waiter(1),
    );
    let first = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };
    let timed_out = engine.complete(&key, first.fence(), ReadNetworkOutcome::TimedOut);

    assert_eq!(timed_out.disposition(), ReadCompletionDisposition::TimedOut);
    assert_eq!(timed_out.settlements().len(), 1);
    assert_eq!(timed_out.settlements()[0].waiter().get(), 1);
    assert_eq!(
        timed_out.settlements()[0].terminal(),
        ReadWaiterTerminal::TimedOut
    );
    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), 0);

    let second = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a retry start, got {other:?}"),
    };
    let failed = engine.complete(&key, second.fence(), failed());

    assert_eq!(failed.disposition(), ReadCompletionDisposition::Failed);
    assert!(failed.settlements().is_empty());
    assert_eq!(engine.candidate_count(&key), 1);
    assert!(matches!(engine.wake(&key), ReadWakeResult::Start(_)));
}

#[test]
fn duplicate_wake_does_not_allocate_another_operation() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event"),
        waiter(1),
    );
    let first = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };

    assert_eq!(engine.wake(&key), ReadWakeResult::AlreadyActive);
    assert_eq!(engine.active_operation(&key), Some(first.fence()));
    assert_eq!(
        engine.last_operation_generation(),
        first.fence().operation_generation()
    );
}

#[test]
fn session_and_operation_generations_fence_stale_input() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    let stale_admission = engine.admit(
        SESSION - 1,
        key.clone(),
        unordered("synthetic-event"),
        waiter(1),
    );
    assert_eq!(
        stale_admission.status(),
        ReadAdmissionStatus::Rejected(ReadAdmissionRejection::StaleSession)
    );
    assert_eq!(engine.candidate_count(&key), 0);

    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event"),
        waiter(2),
    );
    let operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };
    let stale_fence =
        ReadOperationFence::new(SESSION + 1, operation.fence().operation_generation());
    let stale = engine.complete(&key, stale_fence, ReadNetworkOutcome::Succeeded);

    assert_eq!(
        stale.disposition(),
        ReadCompletionDisposition::StaleDiscarded
    );
    assert_eq!(engine.active_operation(&key), Some(operation.fence()));
    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), 1);
}

#[test]
fn successful_newer_candidate_settles_dominated_waiters_exactly_once() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-old", 9, 5),
        waiter(1),
    );
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-event-new", 9, 6),
        waiter(2),
    );
    let operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };
    let succeeded = engine.complete(&key, operation.fence(), ReadNetworkOutcome::Succeeded);

    assert_eq!(
        succeeded.disposition(),
        ReadCompletionDisposition::Succeeded
    );
    assert_eq!(succeeded.settlements().len(), 2);
    assert!(
        succeeded
            .settlements()
            .iter()
            .all(|settlement| settlement.terminal() == ReadWaiterTerminal::Converged)
    );
    assert_eq!(engine.candidate_count(&key), 0);
    assert_eq!(engine.waiter_count(&key), 0);

    let duplicate = engine.complete(&key, operation.fence(), ReadNetworkOutcome::Succeeded);
    assert_eq!(
        duplicate.disposition(),
        ReadCompletionDisposition::StaleDiscarded
    );
    assert!(duplicate.settlements().is_empty());
}

#[test]
fn public_thread_and_fully_read_keys_can_each_own_one_active_operation() {
    let mut engine = ReadStateEngine::new(SESSION);
    let keys = [
        public("synthetic-room"),
        thread("synthetic-room", "synthetic-root"),
        fully_read("synthetic-room"),
    ];
    for (index, key) in keys.iter().enumerate() {
        engine.admit(
            SESSION,
            key.clone(),
            unordered(&format!("synthetic-event-{index}")),
            waiter(index as u64),
        );
    }

    let starts = keys
        .iter()
        .filter(|key| matches!(engine.wake(key), ReadWakeResult::Start(_)))
        .count();

    assert_eq!(starts, 3);
    assert_eq!(engine.active_operation_count(), 3);
}

#[test]
fn persistence_snapshot_restores_only_desired_targets_without_waiters_or_positions() {
    let public_key = public("secret-room");
    let thread_key = thread("secret-room", "secret-root");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        public_key.clone(),
        positioned("secret-public-event", 12, 8),
        waiter(1),
    );
    engine.admit(
        SESSION,
        thread_key.clone(),
        unordered("secret-thread-event"),
        waiter(2),
    );

    let snapshot = engine.persistence_snapshot();
    assert_eq!(snapshot.entry_count(), 2);
    let rendered = format!("{snapshot:?}");
    assert!(!rendered.contains("secret-room"));
    assert!(!rendered.contains("secret-root"));
    assert!(!rendered.contains("secret-public-event"));

    let restored = ReadStateEngine::restore(SESSION + 1, snapshot)
        .expect("bounded manager snapshot must restore");
    assert_eq!(restored.session_generation(), SESSION + 1);
    assert!(restored.has_candidate(&public_key, "secret-public-event"));
    assert!(restored.has_candidate(&thread_key, "secret-thread-event"));
    assert_eq!(restored.waiter_count(&public_key), 0);
    assert_eq!(restored.waiter_count(&thread_key), 0);
    assert_eq!(
        match restored.keys.get(&public_key) {
            Some(state) => state
                .desired
                .as_ref()
                .expect("restored desired target")
                .target
                .position(),
            None => panic!("public desired target must restore"),
        },
        None,
        "the actor-owned position index is never serialized"
    );
}

#[test]
fn persisted_receipt_policy_filters_public_and_thread_but_keeps_private_fully_read() {
    let public_key = public("policy-room");
    let thread_key = thread("policy-room", "policy-root");
    let fully_read_key = fully_read("policy-room");
    let mut engine = ReadStateEngine::new(SESSION);
    for (index, key) in [
        public_key.clone(),
        thread_key.clone(),
        fully_read_key.clone(),
    ]
    .into_iter()
    .enumerate()
    {
        engine.admit(
            SESSION,
            key,
            unordered(&format!("policy-event-{index}")),
            waiter(index as u64),
        );
    }

    let mut snapshot = engine.persistence_snapshot();
    assert!(snapshot.apply_receipt_policy(false));
    assert_eq!(snapshot.entry_count(), 1);
    assert_eq!(snapshot.entries()[0].key(), &fully_read_key);
    assert!(!snapshot.apply_receipt_policy(false));
    assert!(!snapshot.apply_receipt_policy(true));
}

#[test]
fn authoritative_server_ahead_observation_converges_waiters_and_cancels_stale_worker() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-old", 14, 4),
        waiter(1),
    );
    engine.admit(
        SESSION,
        key.clone(),
        positioned("synthetic-new", 14, 5),
        waiter(2),
    );
    let operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected an active retry, got {other:?}"),
    };

    let reconciliation =
        engine.confirm_authoritative(SESSION, &key, positioned("synthetic-server-ahead", 14, 6));

    assert_eq!(
        reconciliation.superseded_operation(),
        Some(operation.fence())
    );
    assert_eq!(reconciliation.settlements().len(), 2);
    assert!(
        reconciliation
            .settlements()
            .iter()
            .all(|settlement| settlement.terminal() == ReadWaiterTerminal::Converged)
    );
    assert_eq!(engine.candidate_count(&key), 0);
    assert_eq!(engine.active_operation(&key), Some(operation.fence()));

    let stale = engine.complete(&key, operation.fence(), ReadNetworkOutcome::Succeeded);
    assert_eq!(
        stale.disposition(),
        ReadCompletionDisposition::StaleDiscarded
    );
    assert!(stale.settlements().is_empty());
}

#[test]
fn diagnostic_views_and_debug_output_do_not_expose_identifiers() {
    let key = public("secret-room");
    let mut engine = ReadStateEngine::new(SESSION);
    let admission = engine.admit(SESSION, key.clone(), unordered("secret-event"), waiter(1));
    let operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };
    let completion = engine.complete(&key, operation.fence(), ReadNetworkOutcome::TimedOut);

    for rendered in [
        format!("{:?}", admission.diagnostic()),
        format!("{:?}", completion.diagnostic()),
        format!("{key:?}"),
        format!("{operation:?}"),
    ] {
        assert!(!rendered.contains("secret-room"));
        assert!(!rendered.contains("secret-event"));
    }
}

#[test]
fn unordered_latest_admission_replaces_the_older_desired_target() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-a"),
        waiter(1),
    );
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-b"),
        waiter(2),
    );

    assert_eq!(engine.candidate_count(&key), 1);
    assert_eq!(engine.waiter_count(&key), 2);
    assert!(!engine.has_candidate(&key, "synthetic-event-a"));
    assert!(engine.has_candidate(&key, "synthetic-event-b"));
}

#[test]
fn persistence_snapshot_writes_only_the_newest_unordered_target() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-a"),
        waiter(1),
    );
    engine.admit(SESSION, key, unordered("synthetic-event-b"), waiter(2));

    let snapshot = engine.persistence_snapshot();
    assert_eq!(snapshot.candidate_count(), 1);
    assert_eq!(snapshot.entries()[0].event_ids(), ["synthetic-event-b"]);
}

#[test]
fn failed_latest_target_is_retained_without_replaying_an_older_target() {
    let key = public("synthetic-room");
    let mut engine = ReadStateEngine::new(SESSION);
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-a"),
        waiter(1),
    );
    engine.admit(
        SESSION,
        key.clone(),
        unordered("synthetic-event-b"),
        waiter(2),
    );

    let operation = match engine.wake(&key) {
        ReadWakeResult::Start(operation) => operation,
        other => panic!("expected a start, got {other:?}"),
    };
    assert_eq!(operation.target().event_id(), "synthetic-event-b");
    engine.complete(&key, operation.fence(), failed());

    assert_eq!(engine.candidate_count(&key), 1);
    assert!(engine.has_candidate(&key, "synthetic-event-b"));
    assert!(!engine.has_candidate(&key, "synthetic-event-a"));
}

#[test]
fn a_confirmed_boundary_only_advances_on_provable_order() {
    let at = |rank: u64| {
        Some(ReadPositionEvidence {
            generation: 7,
            rank,
        })
    };
    // Nothing confirmed yet: the first target is the only boundary there is.
    assert!(confirmed_boundary_accepts(None, ("$first", at(5))));
    // The same target again is idempotent, whatever the index says.
    for incoming in [at(5), at(9), None] {
        assert!(!confirmed_boundary_accepts(
            Some(("$first", at(5))),
            ("$first", incoming)
        ));
    }
    // A strictly later rank in the same index generation is proof.
    assert!(confirmed_boundary_accepts(
        Some(("$older", at(2))),
        ("$newer", at(5))
    ));
    // An earlier rank, an equal rank and a rebuilt generation are not.
    assert!(!confirmed_boundary_accepts(
        Some(("$newer", at(5))),
        ("$older", at(2))
    ));
    assert!(!confirmed_boundary_accepts(
        Some(("$newer", at(5))),
        ("$other", at(5))
    ));
    assert!(!confirmed_boundary_accepts(
        Some(("$older", at(2))),
        (
            "$newer",
            Some(ReadPositionEvidence {
                generation: 8,
                rank: 5
            })
        )
    ));
    // A target the current index cannot place is incomparable: keep the boundary
    // rather than guess from the id or from arrival order.
    assert!(!confirmed_boundary_accepts(
        Some(("$older", at(2))),
        ("$outside", None)
    ));
    assert!(!confirmed_boundary_accepts(
        Some(("$evicted", None)),
        ("$newer", at(5))
    ));
}
