use super::{
    record_initial_share_diagnostic, record_initial_share_session_diagnostic,
    record_room_key_rotation_diagnostic,
};
use koushi_diagnostics::{DiagnosticCounterContext, test_support};
use matrix_sdk::encryption::{
    InitialShareDeviceClass as Class, InitialShareDeviceDiagnostic, InitialShareSessionDiagnostic,
    InitialShareStage as Stage, RoomKeyCreationOutcome, RoomKeyDiagnosticAlias,
    RoomKeyFirstShareOutcome, RoomKeyRotationDiagnostic, RoomKeyRotationReason,
};

#[test]
fn first_event_updates_only_the_matching_rotation_boundary() {
    let _guard = test_support::lock();
    let counters = DiagnosticCounterContext::new();
    koushi_diagnostics::reset_rotation_ledger();
    for session in [8, 9] {
        record_room_key_rotation_diagnostic(
            &counters,
            RoomKeyRotationDiagnostic {
                room: RoomKeyDiagnosticAlias::new(session - 7),
                previous_session: None,
                new_session: Some(RoomKeyDiagnosticAlias::new(session)),
                reason: RoomKeyRotationReason::Initial,
                creation_outcome: RoomKeyCreationOutcome::Created,
                first_share_outcome: RoomKeyFirstShareOutcome::Pending,
                first_send_correlation_present: false,
                discard_elapsed_ms: None,
                elapsed_ms: 1,
            },
        );
    }

    record_initial_share_session_diagnostic(
        &counters,
        InitialShareSessionDiagnostic {
            session: RoomKeyDiagnosticAlias::new(8),
            first_event_message_index: 0,
            all_initial_shares_settled_first: true,
            pending_requests_bucket: 0,
            eligible_own_devices: 1,
            eligible_peer_devices: 1,
            index0_shares_committed: 2,
            after_index0_shares_committed: 0,
            homeserver_accepted_devices: 2,
            created_at_index0: true,
            elapsed_ms: 2,
        },
    );

    let snapshot = test_support::rotation_snapshot();
    assert_eq!(snapshot.records.len(), 2);
    for (record, expected) in snapshot.records.iter().zip([true, false]) {
        assert!(record.event.fields.iter().any(|field| {
            field.key == "first_send_correlation_present"
                && field.value == koushi_diagnostics::DiagnosticValue::Boolean(expected)
        }));
    }
}

fn counter_value(counters: &DiagnosticCounterContext, name: &'static str) -> u64 {
    let snapshot = counters.snapshot();
    snapshot
        .records
        .iter()
        .find(|record| {
            record.event.source == "core.room_key_summary"
                && record.event.fields.iter().any(|field| {
                    field.key == "name"
                        && field.value == koushi_diagnostics::DiagnosticValue::Token(name)
                })
        })
        .and_then(|record| {
            record
                .event
                .fields
                .iter()
                .find_map(|field| match field.value {
                    koushi_diagnostics::DiagnosticValue::Count(count) if field.key == "count" => {
                        Some(count)
                    }
                    _ => None,
                })
        })
        .unwrap_or(0)
}

fn device_event(class: Class, stage: Stage) -> InitialShareDeviceDiagnostic {
    InitialShareDeviceDiagnostic {
        session: RoomKeyDiagnosticAlias::new(7),
        device: RoomKeyDiagnosticAlias::new(3),
        device_class: class,
        stage,
        elapsed_ms: 12,
    }
}

#[test]
fn initial_share_diagnostic_records_closed_tokens_and_counters() {
    let _guard = test_support::lock();
    let counters = DiagnosticCounterContext::new();
    let diagnostic_start = test_support::detail_cursor();

    record_initial_share_diagnostic(
        &counters,
        device_event(Class::VerifiedPeer, Stage::Eligible),
    );
    record_initial_share_diagnostic(&counters, device_event(Class::Unknown, Stage::OlmMissing));
    record_initial_share_diagnostic(&counters, device_event(Class::Unknown, Stage::OlmEncrypted));
    record_initial_share_diagnostic(
        &counters,
        device_event(Class::Unknown, Stage::OlmEncryptionFailed),
    );
    record_initial_share_diagnostic(&counters, device_event(Class::Unknown, Stage::Withheld));
    record_initial_share_diagnostic(
        &counters,
        device_event(Class::Unknown, Stage::RequestQueued),
    );
    record_initial_share_diagnostic(
        &counters,
        device_event(Class::Unknown, Stage::HomeserverAccepted),
    );
    record_initial_share_diagnostic(
        &counters,
        device_event(
            Class::Unknown,
            Stage::ShareStateCommitted { message_index: 0 },
        ),
    );
    record_initial_share_diagnostic(
        &counters,
        device_event(
            Class::Unknown,
            Stage::ShareStateCommitted { message_index: 4 },
        ),
    );
    record_initial_share_session_diagnostic(
        &counters,
        InitialShareSessionDiagnostic {
            session: RoomKeyDiagnosticAlias::new(7),
            first_event_message_index: 0,
            all_initial_shares_settled_first: true,
            pending_requests_bucket: 0,
            eligible_own_devices: 0,
            eligible_peer_devices: 1,
            index0_shares_committed: 1,
            after_index0_shares_committed: 1,
            homeserver_accepted_devices: 1,
            created_at_index0: true,
            elapsed_ms: 12,
        },
    );

    assert_eq!(counter_value(&counters, "initial_share_eligible_peer"), 1);
    assert_eq!(counter_value(&counters, "initial_share_eligible_own"), 0);
    assert_eq!(counter_value(&counters, "initial_share_olm_missing"), 1);
    assert_eq!(counter_value(&counters, "initial_share_olm_encrypted"), 1);
    assert_eq!(
        counter_value(&counters, "initial_share_olm_encryption_failed"),
        1
    );
    assert_eq!(counter_value(&counters, "initial_share_withheld"), 1);
    assert_eq!(counter_value(&counters, "initial_share_request_queued"), 1);
    assert_eq!(
        counter_value(&counters, "initial_share_homeserver_accepted"),
        1
    );
    assert_eq!(
        counter_value(&counters, "initial_share_share_committed_index0"),
        1
    );
    assert_eq!(
        counter_value(&counters, "initial_share_share_committed_after_index0"),
        1
    );
    assert_eq!(
        counter_value(&counters, "initial_share_first_event_all_settled"),
        1
    );
    assert_eq!(
        counter_value(&counters, "initial_share_first_event_pending"),
        0
    );
    assert_eq!(
        counter_value(&counters, "initial_share_sessions_at_index0"),
        1
    );
    assert_eq!(
        counter_value(&counters, "initial_share_sessions_after_index0"),
        0
    );

    let records = test_support::detail_records_since(diagnostic_start);
    let stage_records: Vec<_> = records
        .iter()
        .filter(|record| record.event.source == "core.initial_share")
        .collect();
    // 9 device stages + 1 session summary.
    assert_eq!(stage_records.len(), 10);
    let stage_tokens: Vec<_> = stage_records
        .iter()
        .filter(|record| record.event.stage == "stage")
        .map(|record| {
            record
                .event
                .fields
                .iter()
                .find(|field| field.key == "stage")
                .and_then(|field| match &field.value {
                    koushi_diagnostics::DiagnosticValue::Token(token) => Some(*token),
                    _ => None,
                })
                .expect("stage token")
        })
        .collect();
    for token in [
        "eligible",
        "olm_missing",
        "olm_encrypted",
        "olm_encryption_failed",
        "withheld",
        "request_queued",
        "homeserver_accepted",
        "share_state_committed",
        "share_state_committed",
    ] {
        assert!(stage_tokens.contains(&token), "missing stage token {token}");
    }
}

#[test]
fn initial_share_diagnostics_never_expose_private_values() {
    let _guard = test_support::lock();
    let counters = DiagnosticCounterContext::new();
    let diagnostic_start = test_support::detail_cursor();

    record_initial_share_diagnostic(
        &counters,
        device_event(Class::VerifiedPeer, Stage::Eligible),
    );
    record_initial_share_diagnostic(
        &counters,
        device_event(
            Class::Unknown,
            Stage::ShareStateCommitted { message_index: 0 },
        ),
    );
    record_initial_share_session_diagnostic(
        &counters,
        InitialShareSessionDiagnostic {
            session: RoomKeyDiagnosticAlias::new(7),
            first_event_message_index: 0,
            all_initial_shares_settled_first: true,
            pending_requests_bucket: 0,
            eligible_own_devices: 1,
            eligible_peer_devices: 2,
            index0_shares_committed: 1,
            after_index0_shares_committed: 0,
            homeserver_accepted_devices: 1,
            created_at_index0: true,
            elapsed_ms: 12,
        },
    );

    for record in test_support::detail_records_since(diagnostic_start) {
        let text = format!("{:?}", record.event);
        assert!(
            !text.contains('@') && !text.contains('!') && !text.contains("http"),
            "privacy leak in initial-share diagnostic: {text}"
        );
        assert!(!text.contains("session_key"), "privacy leak: {text}");
        assert!(!text.contains("ciphertext"), "privacy leak: {text}");
    }
}

#[test]
fn initial_share_counters_survive_detail_ring_eviction() {
    let _guard = test_support::lock();
    let counters = DiagnosticCounterContext::new();

    // The aggregate counter lives outside the bounded detail ring: emit
    // without recording any detail and confirm the counter still exports.
    let detail_before = test_support::detail_cursor();
    counters.increment("initial_share_olm_encrypted");
    assert!(
        !test_support::detail_records_since(detail_before)
            .iter()
            .any(|record| record.event.source == "core.room_key_summary"),
        "the counter must not consume detail-ring capacity"
    );
    assert_eq!(counter_value(&counters, "initial_share_olm_encrypted"), 1);
}
