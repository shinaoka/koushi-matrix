use std::{sync::Arc, time::Duration};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
#[cfg(any(test, feature = "test-hooks"))]
use koushi_sdk::MatrixClientSession;
#[cfg(any(test, feature = "test-hooks"))]
use koushi_state::VerificationTarget;
use koushi_state::{AppAction, SessionInfo};
#[cfg(any(test, feature = "test-hooks"))]
use matrix_sdk::test_utils::mocks::MatrixMockServer;

use tokio::sync::{broadcast, mpsc, oneshot};

#[cfg(any(test, feature = "test-hooks"))]
use super::refresh_device_keys_and_assert_known;
use super::{
    PendingTrustTransition, TrustLifecycleDecision, VerificationMethodDiscoveryResult,
    active_own_user_sas_flow_for_provisional_encryption_sync, advance_observed_trust,
    begin_provisional_encryption_sync_cursor_attempt, current_session_status_completion_action,
    current_session_status_connectivity_proven, current_session_status_failure,
    current_session_status_observed_non_verified_trust, current_session_status_settled_event,
    first_provisional_encryption_sync_is_current, method_discovery_admission_timeout_is_current,
    method_discovery_is_current, own_user_sas_recheck_is_current,
    record_verification_admission_event, record_verification_method_discovery_event,
    recovery_sync_should_resume, retry_should_restart_method_discovery,
    run_recovery_state_observation, should_discover_verification_methods, trust_lifecycle_decision,
    trust_projection_ack_matches, unknown_verification_gate, verification_method_discovery_event,
    wait_for_verification_method_discovery,
};
use crate::account::actor::AccountMessage;
use crate::account::test_support::{
    KeyQueryControl, acknowledge_next_verified_projection,
    consume_initial_unknown_trust_projection, inspect_session_runtime, inspect_sync_owners,
    login_gated_actor, login_gated_actor_at, recv_account_action_with_sliding_sync_effects,
    spawn_named_quarantine_password_server_with_controls,
};

use crate::executor;
use koushi_protocol::event::{AccountEvent, CoreEvent};

use koushi_protocol::ids::AccountKey;

use futures_util::stream;

fn session_status_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://private.example.test".to_owned(),
        user_id: "@private:example.test".to_owned(),
        device_id: "PRIVATE-DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
    }
}

fn verified_session_inspection() -> koushi_sdk::MatrixCurrentSessionInspection {
    koushi_sdk::MatrixCurrentSessionInspection {
        device_display_name: Some("Private Device Name".to_owned()),
        verification: koushi_state::CurrentDeviceTrustState::Verified,
        is_cross_signed_by_owner: true,
        own_identity_verification: koushi_state::OwnIdentityVerification::Verified,
        key_backup: koushi_state::CurrentSessionBackupState::Ready,
    }
}

#[test]
fn session_status_completion_requires_request_and_session_generation_fences() {
    let info = session_status_info();
    for (active_request, active_generation, promoted) in
        [(Some(8), 4, true), (Some(7), 5, true), (Some(7), 4, false)]
    {
        assert!(
            current_session_status_completion_action(
                active_request,
                active_generation,
                promoted,
                Some(&info),
                7,
                4,
                koushi_state::CurrentSessionSyncState::Running,
                Ok(verified_session_inspection()),
                123,
            )
            .is_none(),
            "stale request, stale generation, and cleared sessions must all be rejected"
        );
    }
}

#[test]
fn session_status_non_verified_observation_routes_to_the_trust_gate() {
    for trust in [
        koushi_state::CurrentDeviceTrustState::Unknown,
        koushi_state::CurrentDeviceTrustState::Unverified,
    ] {
        let mut inspection = verified_session_inspection();
        inspection.verification = trust;
        assert_eq!(
            current_session_status_observed_non_verified_trust(&Ok(inspection)),
            Some(trust)
        );
    }
    assert_eq!(
        current_session_status_observed_non_verified_trust(&Ok(verified_session_inspection())),
        None
    );
}

#[test]
fn session_status_probe_requires_proven_running_sync() {
    use koushi_state::CurrentSessionSyncState;

    assert!(current_session_status_connectivity_proven(
        CurrentSessionSyncState::Running
    ));
    for state in [
        CurrentSessionSyncState::Stopped,
        CurrentSessionSyncState::Starting,
        CurrentSessionSyncState::Error,
    ] {
        assert!(!current_session_status_connectivity_proven(state));
    }
}

#[test]
fn session_status_request_failures_keep_auth_network_server_and_sdk_distinct() {
    use koushi_sdk::MatrixCurrentSessionInspectionError as Inspection;
    use koushi_state::CurrentSessionStatusFailureKind as Failure;

    for (inspection, expected) in [
        (Inspection::Authentication, Failure::Authentication),
        (Inspection::Network, Failure::Network),
        (Inspection::Server, Failure::Server),
        (Inspection::DeviceRequest, Failure::Sdk),
        (Inspection::IdentityRequest, Failure::Sdk),
        (Inspection::Unavailable, Failure::Unavailable),
        (Inspection::CurrentDeviceMissing, Failure::Unavailable),
    ] {
        assert_eq!(current_session_status_failure(inspection), expected);
    }
}

#[test]
fn session_status_sdk_failure_projects_coarse_failed_action() {
    let info = session_status_info();
    assert_eq!(
        current_session_status_completion_action(
            Some(7),
            4,
            true,
            Some(&info),
            7,
            4,
            koushi_state::CurrentSessionSyncState::Error,
            Err(koushi_state::CurrentSessionStatusFailureKind::Sdk),
            123,
        ),
        Some(AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 7,
            kind: koushi_state::CurrentSessionStatusFailureKind::Sdk,
            checked_at_ms: 123,
        })
    );
}

#[test]
fn session_status_success_uses_durable_auth_sync_and_sdk_trust_facts() {
    let info = session_status_info();
    let Some(AppAction::CurrentSessionStatusRefreshed {
        request_id,
        details,
    }) = current_session_status_completion_action(
        Some(7),
        4,
        true,
        Some(&info),
        7,
        4,
        koushi_state::CurrentSessionSyncState::Running,
        Ok(verified_session_inspection()),
        123,
    )
    else {
        panic!("current completion should project ready details");
    };
    assert_eq!(request_id, 7);
    assert_eq!(details.device_id, "PRIVATE-DEVICE");
    assert_eq!(
        details.authentication_method,
        koushi_state::SessionAuthenticationMethod::OAuth
    );
    assert_eq!(
        details.sync_state,
        koushi_state::CurrentSessionSyncState::Running
    );
    assert_eq!(
        details.verification,
        koushi_state::CurrentDeviceTrustState::Verified
    );
}

#[test]
fn session_status_diagnostics_expose_only_coarse_result_and_elapsed_time() {
    let info = session_status_info();
    let action = current_session_status_completion_action(
        Some(7),
        4,
        true,
        Some(&info),
        7,
        4,
        koushi_state::CurrentSessionSyncState::Running,
        Ok(verified_session_inspection()),
        123,
    )
    .expect("current completion");
    let formatted = koushi_diagnostics::format_event(&current_session_status_settled_event(
        &action,
        Duration::from_millis(9),
    ));
    assert_eq!(
        formatted,
        "stage=refresh_settled elapsed_ms=9 result=ready verdict=verified"
    );
    let deferred = AppAction::CurrentSessionStatusRefreshFailed {
        request_id: 8,
        kind: koushi_state::CurrentSessionStatusFailureKind::ConnectivityUnavailable,
        checked_at_ms: 124,
    };
    assert_eq!(
        koushi_diagnostics::format_event(&current_session_status_settled_event(
            &deferred,
            Duration::from_millis(3),
        )),
        "stage=refresh_settled elapsed_ms=3 result=connectivity_unavailable"
    );
    for private in [
        "private.example.test",
        "@private:example.test",
        "PRIVATE-DEVICE",
        "Private Device Name",
    ] {
        assert!(!formatted.contains(private));
    }
}

#[test]
fn method_discovery_rejects_stale_generation_serial_and_missing_session() {
    assert!(method_discovery_is_current(4, 4, 9, 9, true));
    assert!(!method_discovery_is_current(3, 4, 9, 9, true));
    assert!(!method_discovery_is_current(4, 4, 8, 9, true));
    assert!(!method_discovery_is_current(4, 4, 9, 9, false));
}

#[tokio::test]
async fn verification_method_discovery_times_out_pending_sdk_work() {
    let result =
        wait_for_verification_method_discovery(Duration::from_millis(1), std::future::pending())
            .await;

    assert_eq!(
        result,
        VerificationMethodDiscoveryResult::Failed(
            koushi_state::VerificationGateFailureKind::Timeout
        )
    );
}

#[tokio::test]
async fn verification_method_discovery_maps_known_and_unknown_gate_results() {
    let known = koushi_state::VerificationGateState {
        methods: vec![koushi_state::VerificationMethodCapability::RecoveryKey],
        account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
        failure: None,
    };
    assert_eq!(
        wait_for_verification_method_discovery(
            Duration::from_secs(1),
            std::future::ready(known.clone()),
        )
        .await,
        VerificationMethodDiscoveryResult::Discovered(known)
    );
    assert_eq!(
        wait_for_verification_method_discovery(
            Duration::from_secs(1),
            std::future::ready(unknown_verification_gate()),
        )
        .await,
        VerificationMethodDiscoveryResult::Failed(koushi_state::VerificationGateFailureKind::Sdk)
    );
}

#[test]
fn verification_method_discovery_retry_can_arm_a_cold_unverified_provisional_session() {
    assert!(retry_should_restart_method_discovery(
        false,
        Some(koushi_state::CurrentDeviceTrustState::Unverified),
    ));
    assert!(!retry_should_restart_method_discovery(
        true,
        Some(koushi_state::CurrentDeviceTrustState::Unverified),
    ));
    assert!(!retry_should_restart_method_discovery(
        false,
        Some(koushi_state::CurrentDeviceTrustState::Unknown),
    ));
    assert!(!retry_should_restart_method_discovery(
        false,
        Some(koushi_state::CurrentDeviceTrustState::Verified),
    ));
    assert!(!retry_should_restart_method_discovery(false, None));
}

#[test]
fn admission_timeout_requires_the_current_cold_provisional_generation() {
    assert!(method_discovery_admission_timeout_is_current(
        4, 4, 9, 9, true, false, false,
    ));
    assert!(!method_discovery_admission_timeout_is_current(
        3, 4, 9, 9, true, false, false,
    ));
    assert!(!method_discovery_admission_timeout_is_current(
        4, 4, 8, 9, true, false, false,
    ));
    assert!(!method_discovery_admission_timeout_is_current(
        4, 4, 9, 9, false, false, false,
    ));
    assert!(!method_discovery_admission_timeout_is_current(
        4, 4, 9, 9, true, true, false,
    ));
    assert!(!method_discovery_admission_timeout_is_current(
        4, 4, 9, 9, true, false, true,
    ));
}

#[test]
fn provisional_encryption_sync_attempt_starts_only_without_an_active_owner() {
    assert!(begin_provisional_encryption_sync_cursor_attempt(false));
    assert!(!begin_provisional_encryption_sync_cursor_attempt(true));
}

#[test]
fn recovery_resumes_provisional_encryption_sync_only_for_the_current_gated_session() {
    assert!(recovery_sync_should_resume(4, 4, false, false));
    assert!(!recovery_sync_should_resume(3, 4, false, false));
    assert!(!recovery_sync_should_resume(4, 4, true, false));
    assert!(!recovery_sync_should_resume(4, 4, false, true));
}

#[test]
fn first_provisional_encryption_sync_ack_rejects_stale_torn_down_and_promoted_sessions() {
    assert!(first_provisional_encryption_sync_is_current(
        4, 4, true, false
    ));
    assert!(!first_provisional_encryption_sync_is_current(
        3, 4, true, false
    ));
    assert!(!first_provisional_encryption_sync_is_current(
        4, 4, false, false
    ));
    assert!(!first_provisional_encryption_sync_is_current(
        4, 4, true, true
    ));
    assert_eq!(unknown_verification_gate().methods, Vec::new());
    assert_eq!(
        unknown_verification_gate().account_kind,
        koushi_state::VerificationAccountKind::Unknown
    );
}

#[test]
fn provisional_encryption_sync_rechecks_only_current_unstarted_own_user_flow() {
    assert!(own_user_sas_recheck_is_current(
        4, 4, true, false, true, false
    ));
    assert!(!own_user_sas_recheck_is_current(
        3, 4, true, false, true, false
    ));
    assert!(!own_user_sas_recheck_is_current(
        4, 4, false, false, true, false
    ));
    assert!(!own_user_sas_recheck_is_current(
        4, 4, true, true, true, false
    ));
    assert!(!own_user_sas_recheck_is_current(
        4, 4, true, false, false, false
    ));
    assert!(!own_user_sas_recheck_is_current(
        4, 4, true, false, true, true
    ));
}

#[test]
fn provisional_encryption_sync_diagnostics_require_current_own_user_flow() {
    assert_eq!(
        active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, true, false, Some(73)),
        Some(73)
    );
    assert_eq!(
        active_own_user_sas_flow_for_provisional_encryption_sync(3, 4, true, false, Some(73)),
        None
    );
    assert_eq!(
        active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, false, false, Some(73)),
        None
    );
    assert_eq!(
        active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, true, true, Some(73)),
        None
    );
    assert_eq!(
        active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, true, false, None),
        None
    );
}

#[test]
fn unknown_trust_does_not_discover_verification_methods() {
    assert!(!should_discover_verification_methods(
        koushi_state::CurrentDeviceTrustState::Unknown
    ));
    assert!(should_discover_verification_methods(
        koushi_state::CurrentDeviceTrustState::Unverified
    ));
    assert!(!should_discover_verification_methods(
        koushi_state::CurrentDeviceTrustState::Verified
    ));
}

#[test]
fn observed_trust_suppresses_duplicates_but_preserves_real_transitions() {
    let mut last = koushi_state::CurrentDeviceTrustState::Unverified;

    assert!(!advance_observed_trust(
        &mut last,
        koushi_state::CurrentDeviceTrustState::Unverified,
    ));
    assert!(advance_observed_trust(
        &mut last,
        koushi_state::CurrentDeviceTrustState::Verified,
    ));
    assert!(!advance_observed_trust(
        &mut last,
        koushi_state::CurrentDeviceTrustState::Verified,
    ));
    assert!(advance_observed_trust(
        &mut last,
        koushi_state::CurrentDeviceTrustState::Unverified,
    ));
}

#[test]
fn verification_admission_diagnostic_is_mirrored_to_privacy_safe_stderr() {
    let output = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .args([
        "--exact",
        "account::trust_gate::tests::verification_admission_diagnostic_child",
        "--ignored",
        "--nocapture",
    ])
    .output()
    .expect("verification admission diagnostic child should run");
    assert!(output.status.success(), "child failed: {output:?}");

    let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
    assert!(stderr.contains("core.verification_admission"));
    assert!(stderr.contains("stage=trust_read_finished"));
    assert!(!stderr.contains('@'));
    assert!(!stderr.contains("access_token"));

    let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
    let snapshot: serde_json::Value = serde_json::from_str(
        stdout
            .lines()
            .find(|line| line.starts_with('{'))
            .expect("child should print one JSON snapshot"),
    )
    .expect("child output should be a JSON snapshot");
    assert!(snapshot["records"].as_array().is_some_and(|records| {
        records.iter().any(|record| {
            record["event"]["source"] == "core.verification_admission"
                && record["event"]["stage"] == "trust_read_finished"
        })
    }));
}

#[test]
#[ignore]
fn verification_admission_diagnostic_child() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_verification_admission_event(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.verification_admission",
            "trust_read_finished",
        )
        .field(DiagnosticField::count("generation", 7))
        .field(DiagnosticField::count("transition_id", 0))
        .field(DiagnosticField::token("trust", "verified"))
        .field(DiagnosticField::milliseconds("elapsed_ms", 42)),
    );
    println!(
        "{}",
        serde_json::to_string(&koushi_diagnostics::test_support::detail_snapshot())
            .expect("diagnostic snapshot should serialize")
    );
}

#[test]
fn verification_method_discovery_diagnostic_is_mirrored_to_privacy_safe_stderr() {
    let output = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .args([
        "--exact",
        "account::trust_gate::tests::verification_method_discovery_diagnostic_child",
        "--ignored",
        "--nocapture",
    ])
    .output()
    .expect("verification method discovery diagnostic child should run");
    assert!(output.status.success(), "child failed: {output:?}");

    let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
    assert!(stderr.contains("core.verification_method_discovery"));
    assert!(stderr.contains("stage=finished"));
    assert!(!stderr.contains('@'));
    assert!(!stderr.contains("access_token"));

    let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
    let snapshot: serde_json::Value = serde_json::from_str(
        stdout
            .lines()
            .find(|line| line.starts_with('{'))
            .expect("child should print one JSON snapshot"),
    )
    .expect("child output should be a JSON snapshot");
    assert!(snapshot["records"].as_array().is_some_and(|records| {
        records.iter().any(|record| {
            record["event"]["source"] == "core.verification_method_discovery"
                && record["event"]["stage"] == "finished"
        })
    }));
}

#[test]
#[ignore]
fn verification_method_discovery_diagnostic_child() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_verification_method_discovery_event(
        verification_method_discovery_event("finished", 7, 11)
            .field(DiagnosticField::token("outcome", "failed"))
            .field(DiagnosticField::milliseconds("elapsed_ms", 42)),
    );
    println!(
        "{}",
        serde_json::to_string(&koushi_diagnostics::test_support::detail_snapshot())
            .expect("diagnostic snapshot should serialize")
    );
}

#[test]
fn trust_lifecycle_is_generation_safe_and_fail_closed() {
    use koushi_state::CurrentDeviceTrustState::{Unknown, Unverified, Verified};

    assert_eq!(
        trust_lifecycle_decision(4, 5, false, Verified),
        TrustLifecycleDecision::IgnoreStale
    );
    assert_eq!(
        trust_lifecycle_decision(5, 5, false, Unknown),
        TrustLifecycleDecision::StayGated
    );
    assert_eq!(
        trust_lifecycle_decision(5, 5, false, Unverified),
        TrustLifecycleDecision::StayGated
    );
    assert_eq!(
        trust_lifecycle_decision(5, 5, false, Verified),
        TrustLifecycleDecision::Promote
    );
    assert_eq!(
        trust_lifecycle_decision(5, 5, true, Unverified),
        TrustLifecycleDecision::Gate
    );
    assert_eq!(
        trust_lifecycle_decision(5, 5, true, Unknown),
        TrustLifecycleDecision::Gate
    );
}

#[tokio::test]
async fn duplicate_gate_observations_reuse_one_projection_transition() {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;

    for _ in 0..2 {
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Unverified,
            })
            .await;
    }
    let mut transitions = Vec::new();
    while transitions.len() < 2 {
        let actions = action_rx.recv().await.expect("gate projection action");
        if let [
            AppAction::AuthoritativeDeviceTrustChanged {
                generation,
                transition_id,
                trust: koushi_state::CurrentDeviceTrustState::Unverified,
            },
        ] = actions.as_slice()
        {
            transitions.push((*generation, *transition_id));
        }
    }
    assert_eq!(transitions[0], transitions[1]);
    assert!(
        handle
            .send(AccountMessage::TrustProjectionApplied {
                generation: transitions[0].0,
                transition_id: transitions[0].1,
                ready: false,
                locked: false,
            })
            .await
    );
    executor::timeout(Duration::from_secs(1), async {
        loop {
            if inspect_session_runtime(&handle).await == (true, false, false, true) {
                break;
            }
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("one gated acknowledgement must stop normal children");
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn authoritative_trust_recheck_completion_promotes_through_generation_gated_path() {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;

    handle
        .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
            generation: 2,
            result: Ok(koushi_state::CurrentDeviceTrustState::Verified),
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;

    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, true, true, true)
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn authoritative_trust_recheck_failure_settles_as_retryable_unknown_trust() {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;

    handle
        .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
            generation: 2,
            result: Err(koushi_sdk::CurrentDeviceTrustRecheckError::Sdk),
        })
        .await;

    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::AuthoritativeDeviceTrustChanged {
            trust: koushi_state::CurrentDeviceTrustState::Unknown,
            ..
        }])
    ) {}
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, false, false, true)
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn network_trust_recheck_settlement_records_generation_without_authentication_lock() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();

    handle
        .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
            generation: 2,
            result: Err(koushi_sdk::CurrentDeviceTrustRecheckError::Network),
        })
        .await;
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, true, true, true)
    );

    let expected =
        "stage=trust_recheck_finished_failed generation=2 transition_id=0 failure_kind=network";
    assert!(
        koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
        "missing exact trust-recheck settlement diagnostic: {expected}"
    );
    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !matches!(
                actions.as_slice(),
                [AppAction::SessionAuthenticationInvalidated { .. }]
            ),
            "network trust failure must not emit authentication invalidation"
        );
    }
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn authoritative_trust_recheck_stale_generation_cannot_promote_session() {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;

    handle
        .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
            generation: 1,
            result: Ok(koushi_state::CurrentDeviceTrustState::Verified),
        })
        .await;
    let runtime = inspect_session_runtime(&handle).await;

    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !matches!(
                actions.as_slice(),
                [AppAction::AuthoritativeDeviceTrustChanged {
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                    ..
                }]
            ),
            "a stale recheck completion must not emit a verified projection"
        );
    }
    assert_eq!(runtime, (true, false, false, true));
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn explicit_trust_recheck_is_not_dropped_behind_pending_projection() {
    let (homeserver, query_control) = spawn_counting_quarantine_password_server();
    let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;

    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    let (generation, transition_id) = loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if let [
            AppAction::AuthoritativeDeviceTrustChanged {
                generation,
                transition_id,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            },
        ] = actions.as_slice()
        {
            break (*generation, *transition_id);
        }
    };

    let baseline = query_control
        .count
        .load(std::sync::atomic::Ordering::SeqCst);
    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    handle
        .send(AccountMessage::TrustProjectionApplied {
            generation,
            transition_id,
            ready: false,
            locked: false,
        })
        .await;

    executor::timeout(Duration::from_secs(1), async {
        while query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst)
            == baseline
        {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a projection mismatch must replay the explicit reducer recheck");
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn projection_mismatch_before_explicit_recheck_does_not_block_later_query() {
    let (homeserver, query_control) = spawn_counting_quarantine_password_server();
    let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;

    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    let (generation, transition_id) = loop {
        let actions = action_rx.recv().await.expect("account actions");
        if let [
            AppAction::AuthoritativeDeviceTrustChanged {
                generation,
                transition_id,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            },
        ] = actions.as_slice()
        {
            break (*generation, *transition_id);
        }
    };

    handle
        .send(AccountMessage::TrustProjectionApplied {
            generation,
            transition_id,
            ready: false,
            locked: false,
        })
        .await;
    let baseline = query_control
        .count
        .load(std::sync::atomic::Ordering::SeqCst);
    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;

    executor::timeout(Duration::from_secs(1), async {
        while query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst)
            == baseline
        {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a prior projection mismatch must not block a later explicit recheck");
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn explicit_trust_recheck_arriving_in_flight_is_replayed_after_settlement() {
    let (homeserver, query_control) = spawn_counting_quarantine_password_server();
    let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    query_control
        .hold
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let baseline = query_control
        .count
        .load(std::sync::atomic::Ordering::SeqCst);

    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    executor::timeout(Duration::from_secs(1), async {
        while query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst)
            == baseline
        {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("first authoritative query must start");

    handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
    query_control
        .hold
        .store(false, std::sync::atomic::Ordering::SeqCst);

    executor::timeout(Duration::from_secs(1), async {
        while query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst)
            < baseline + 2
        {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("an in-flight reducer recheck demand must replay after the first query settles");
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn verification_to_normal_sync_handoff_has_one_owner() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let (handle, mut action_rx) = login_gated_actor().await;
    assert!(
        koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .any(|record| {
                record.event.source == "core.verification_admission"
                    && record.event.stage == "provisional_encryption_sync_started"
            }),
        "gated admission must diagnose restricted sync ownership start"
    );
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    handle
        .send(AccountMessage::AttachLifecycleProbe { probe_tx })
        .await;
    assert_eq!(inspect_sync_owners(&handle).await, (true, false, false));
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    assert_eq!(
        inspect_sync_owners(&handle).await,
        (false, false, false),
        "restricted owner must stop before Ready projection acknowledgement"
    );
    assert_eq!(
        probe_rx.try_recv(),
        Ok("provisional_encryption_sync_terminated")
    );
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    assert_eq!(
        inspect_sync_owners(&handle).await,
        (false, false, true),
        "normal sync must be the only owner after Ready acknowledgement"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

fn spawn_counting_quarantine_password_server() -> (String, std::sync::Arc<KeyQueryControl>) {
    let control = std::sync::Arc::new(KeyQueryControl::default());
    let homeserver = spawn_named_quarantine_password_server_with_controls(
        "@fixture-user:example.invalid",
        "FIXTUREDEVICE",
        None,
        Some(std::sync::Arc::clone(&control)),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    (homeserver, control)
}

#[test]
fn stale_projection_ack_does_not_consume_pending_promotion() {
    let pending = PendingTrustTransition {
        generation: 7,
        transition_id: 42,
        decision: TrustLifecycleDecision::Promote,
    };
    assert!(!trust_projection_ack_matches(&pending, 7, 41, false, false));
    assert!(trust_projection_ack_matches(&pending, 7, 42, true, false));
}

#[cfg(any(test, feature = "test-hooks"))]
#[tokio::test]
async fn qa_device_key_refresh_accepts_identityless_exact_device_and_rejects_missing_device() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let (alice, bob) = server.set_up_alice_and_bob_for_encryption().await;
    let bob_target = VerificationTarget {
        user_id: bob.user_id().expect("synthetic Bob user").to_string(),
        device_id: bob.device_id().expect("synthetic Bob device").to_string(),
    };
    let session = MatrixClientSession::from_client_for_testing(
        alice,
        koushi_state::SessionInfo {
            homeserver: server.uri(),
            user_id: "@alice:example.org".to_owned(),
            device_id: "4L1C3".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );

    assert_eq!(
        refresh_device_keys_and_assert_known(&session, bob_target.clone()).await,
        Ok(())
    );
    assert_eq!(
        refresh_device_keys_and_assert_known(
            &session,
            VerificationTarget {
                device_id: "MISSINGDEVICE".to_owned(),
                ..bob_target
            },
        )
        .await,
        Err(())
    );
}

/// Recovery-state observation must emit the reducer-legal state change
/// once per Incomplete transition, even if the stream repeats that state
/// before later becoming Enabled.
#[tokio::test]
async fn recovery_state_observer_deduplicates_repeated_incomplete() {
    let info = SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "DEVICE1".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let account_key = AccountKey(info.user_id.clone());
    let states = stream::iter([
        koushi_state::E2eeRecoveryState::Unknown,
        koushi_state::E2eeRecoveryState::Incomplete,
        koushi_state::E2eeRecoveryState::Incomplete,
        koushi_state::E2eeRecoveryState::Enabled,
        koushi_state::E2eeRecoveryState::Enabled,
    ]);
    let (action_tx, mut action_rx) = mpsc::channel(8);
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let (_stop_tx, stop_rx) = tokio::sync::oneshot::channel();

    run_recovery_state_observation(
        states,
        account_key.clone(),
        action_tx,
        event_tx,
        stop_rx,
        None,
    )
    .await;

    let first_actions = action_rx.recv().await.expect("first action batch");
    assert_eq!(
        first_actions,
        vec![AppAction::E2eeRecoveryStateChanged {
            state: koushi_state::E2eeRecoveryState::Incomplete,
            methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
        }]
    );

    match event_rx.recv().await.expect("recovery event") {
        CoreEvent::Account(AccountEvent::RecoveryRequired {
            account_key: emitted_key,
        }) => {
            assert_eq!(emitted_key, account_key);
        }
        other => panic!("expected RecoveryRequired event, got {other:?}"),
    }

    let second_actions = action_rx.recv().await.expect("follow-up action batch");
    assert_eq!(
        second_actions,
        vec![AppAction::E2eeRecoveryStateChanged {
            state: koushi_state::E2eeRecoveryState::Enabled,
            methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
        }]
    );

    assert!(
        matches!(
            action_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        ),
        "repeated recovery states must not emit duplicate actions"
    );
    assert!(
        matches!(
            event_rx.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ),
        "repeated recovery states must not emit duplicate RecoveryRequired events"
    );
}

#[tokio::test]
async fn recovery_state_observer_stop_interrupts_blocked_action_delivery() {
    let states = stream::iter([koushi_state::E2eeRecoveryState::Incomplete]);
    let (action_tx, mut action_rx) = mpsc::channel(1);
    action_tx
        .send(vec![AppAction::SessionLocked])
        .await
        .expect("fill the reducer action mailbox");
    let (event_tx, mut event_rx) = broadcast::channel(1);
    let (stop_tx, stop_rx) = oneshot::channel();
    let delivery_barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut task = executor::spawn(run_recovery_state_observation(
        states,
        AccountKey("@observer-stop:example.invalid".to_owned()),
        action_tx,
        event_tx,
        stop_rx,
        Some(delivery_barrier.clone()),
    ));

    delivery_barrier.wait().await;
    stop_tx.send(()).expect("request observer stop");
    match executor::timeout(Duration::from_millis(250), &mut task).await {
        Ok(joined) => joined.expect("recovery-state observer task"),
        Err(_) => {
            task.abort();
            let _ = task.await;
            panic!("stop must interrupt a blocked recovery action delivery");
        }
    }

    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::SessionLocked])
    ));
    assert!(
        action_rx.try_recv().is_err(),
        "stop must discard only the blocked observer action"
    );
    assert!(
        event_rx.try_recv().is_err(),
        "a stopped Incomplete delivery must not emit RecoveryRequired"
    );
}
