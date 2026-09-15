use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use koushi_diagnostics::DiagnosticField;

use koushi_sdk::PersistableMatrixSession;
use koushi_state::{
    AppAction, SasEmoji, TrustOperationFailureKind, VerificationCancelReason, VerificationTarget,
};

use tokio::sync::{broadcast, mpsc, oneshot};

use super::{
    INCOMING_VERIFICATION_FLOW_ID_BASE, IncomingVerificationActivity,
    IncomingVerificationObservation, IncomingVerificationRequestDecision, SasAdoptionDecision,
    SasVerificationWaitState, SyntheticVerificationTerminal, VerificationTerminal,
    classify_incoming_verification_request, classify_sas_adoption,
    incoming_verification_request_id, incoming_verification_request_is_current,
    record_sas_verification_event, recovery_failure_token, resolve_sas_adoption,
    run_own_user_sas_start, sas_projection_action, sas_settled_event, sas_state_changed_event,
    sas_state_token, sas_timeout_fired_event, sas_verification_event, sas_waiting_for_token,
    send_observer_output_until_stopped, stop_incoming_verification_observation_with_timeout,
    trust_failure_token, verification_cancel_kind_token, verification_request_state_token,
    verification_terminal_token,
    record_incoming_verification_protection_summary,
    VERIFICATION_PROTECTION_SUMMARY_TRIGGER_RESTORE,
};
use crate::account::actor::{AccountActor, AccountMessage};
use crate::account::recovery_backup::recovery_verification_event;
use crate::account::test_support::{
    acknowledge_next_verified_projection, consume_initial_unknown_trust_projection,
    inspect_session_runtime, login_gated_actor, recv_account_action_with_sliding_sync_effects,
    shutdown_and_ack, spawn_actor_with_dirs, test_request_id,
};
use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
use crate::executor;
use koushi_protocol::command::AccountCommand;
use koushi_protocol::event::CoreEvent;

use crate::link_preview::LinkPreviewContext;
use koushi_protocol::failure::{CoreFailure, RecoveryFailureKind};
use koushi_protocol::ids::RuntimeConnectionId;

use crate::store::StoreActor;
use koushi_store::CredentialStoreBackend;

use tempfile::tempdir;

#[test]
fn own_user_sas_projects_gate_action_while_peer_sas_keeps_peer_projection() {
    let emojis = vec![
        SasEmoji {
            symbol: "x".into(),
            description: "opaque".into()
        };
        7
    ];
    assert!(
        matches!(sas_projection_action(true, 41, emojis.clone()), AppAction::GateSasPresented { flow_id: 41, emojis: projected } if projected == emojis)
    );
    assert!(
        matches!(sas_projection_action(false, 42, emojis.clone()), AppAction::VerificationSasPresented { request_id: 42, emojis: projected } if projected == emojis)
    );
}

#[test]
fn sas_adoption_decision_adopts_once_and_rejects_replay_or_conflict() {
    assert_eq!(classify_sas_adoption(None, 41), SasAdoptionDecision::Adopt);
    assert_eq!(
        classify_sas_adoption(Some(41), 41),
        SasAdoptionDecision::Replay
    );
    assert_eq!(
        classify_sas_adoption(Some(41), 42),
        SasAdoptionDecision::Conflict
    );
}

#[tokio::test]
async fn sas_replay_is_noop_but_conflict_runs_explicit_rejection() {
    let replay_rejections = Arc::new(AtomicU64::new(0));
    let replay = resolve_sas_adoption(Some(41), 41, {
        let replay_rejections = Arc::clone(&replay_rejections);
        move || async move {
            replay_rejections.fetch_add(1, Ordering::SeqCst);
            true
        }
    })
    .await;
    assert_eq!(replay, (SasAdoptionDecision::Replay, None));
    assert_eq!(replay_rejections.load(Ordering::SeqCst), 0);

    let conflict_rejections = Arc::new(AtomicU64::new(0));
    let conflict = resolve_sas_adoption(Some(41), 42, {
        let conflict_rejections = Arc::clone(&conflict_rejections);
        move || async move {
            conflict_rejections.fetch_add(1, Ordering::SeqCst);
            false
        }
    })
    .await;
    assert_eq!(conflict, (SasAdoptionDecision::Conflict, Some(false)));
    assert_eq!(conflict_rejections.load(Ordering::SeqCst), 1);
}

#[test]
fn at_least_once_incoming_transport_uses_target_and_flow_identity() {
    let active_target = VerificationTarget {
        user_id: "@alice:example.test".to_owned(),
        device_id: "ALICE".to_owned(),
    };
    let peer_collision = VerificationTarget {
        user_id: "@mallory:example.test".to_owned(),
        device_id: "MALLORY".to_owned(),
    };
    let device_collision = VerificationTarget {
        user_id: active_target.user_id.clone(),
        device_id: "ALICE-SECOND".to_owned(),
    };
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: Some((&active_target, "stable-flow")),
                sas_active: false,
                own_user_active: false,
            },
            &peer_collision,
            "stable-flow",
        ),
        IncomingVerificationRequestDecision::Conflict,
        "the same opaque flow ID from a different peer/device must be rejected",
    );
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: Some((&active_target, "stable-flow")),
                sas_active: false,
                own_user_active: false,
            },
            &device_collision,
            "stable-flow",
        ),
        IncomingVerificationRequestDecision::Conflict,
        "the same opaque flow ID from a different device must be rejected",
    );
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: Some((&active_target, "stable-flow")),
                sas_active: false,
                own_user_active: false,
            },
            &active_target,
            "stable-flow",
        ),
        IncomingVerificationRequestDecision::Replay,
    );
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: Some((&active_target, "stable-flow")),
                sas_active: false,
                own_user_active: false,
            },
            &active_target,
            "other-flow",
        ),
        IncomingVerificationRequestDecision::Conflict,
    );
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: None,
                sas_active: false,
                own_user_active: false,
            },
            &active_target,
            "new-flow",
        ),
        IncomingVerificationRequestDecision::Adopt,
    );
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: None,
                sas_active: true,
                own_user_active: false,
            },
            &active_target,
            "new-flow",
        ),
        IncomingVerificationRequestDecision::Conflict,
        "an active SAS continuation must continue to reject a new request",
    );
}

#[test]
fn active_own_user_verification_conflicts_with_incoming_request() {
    let incoming_target = VerificationTarget {
        user_id: "@alice:example.test".to_owned(),
        device_id: "ALICE".to_owned(),
    };
    assert_eq!(
        classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request: None,
                sas_active: false,
                own_user_active: true,
            },
            &incoming_target,
            "incoming-flow",
        ),
        IncomingVerificationRequestDecision::Conflict,
        "an own-user verification owns the shared continuation/observer slots",
    );
}

#[test]
fn incoming_verification_transport_rejects_stale_or_sessionless_messages() {
    assert!(incoming_verification_request_is_current(7, 7, true));
    assert!(!incoming_verification_request_is_current(6, 7, true));
    assert!(!incoming_verification_request_is_current(7, 7, false));
}

#[tokio::test]
async fn incoming_verification_mailbox_send_is_stop_aware_when_full() {
    let (sender, mut receiver) = mpsc::channel(1);
    let (_first_stop_tx, mut first_stop_rx) = oneshot::channel();
    assert!(
        send_observer_output_until_stopped(&sender, 1_u8, &mut first_stop_rx,).await,
        "the first ready delivery must fill the product mailbox"
    );
    let (stop_tx, mut stop_rx) = oneshot::channel();
    let blocked_send = executor::spawn(async move {
        send_observer_output_until_stopped(&sender, 2, &mut stop_rx).await
    });
    tokio::task::yield_now().await;

    stop_tx.send(()).expect("request observer stop");
    let delivered = executor::timeout(Duration::from_millis(20), blocked_send)
        .await
        .expect("a stop request must interrupt the full-mailbox send")
        .expect("send task");
    assert!(
        !delivered,
        "a stopped observer must not report the blocked send as delivered"
    );
    assert_eq!(receiver.recv().await, Some(1));
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn incoming_verification_observer_join_has_a_bounded_abort_fallback() {
    let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
    )
    .expect("synthetic session should deserialize");
    let session = koushi_sdk::restore_session(&persistable)
        .await
        .expect("synthetic session should restore");
    let mut observer = koushi_sdk::observe_incoming_verification_requests(&session).await;
    let receiver = observer
        .take_receiver()
        .expect("observer receiver is available once");
    let (stop_tx, _stop_rx) = oneshot::channel();
    let child = executor::spawn(async move {
        let _receiver = receiver;
        std::future::pending::<()>().await
    });
    let child_abort = child.abort_handle();
    let observation = IncomingVerificationObservation {
        stop_tx,
        task: child,
        observer,
    };
    let mut stop = executor::spawn(stop_incoming_verification_observation_with_timeout(
        observation,
        Duration::from_millis(1),
    ));

    let result = executor::timeout(Duration::from_millis(20), &mut stop).await;
    if result.is_err() {
        stop.abort();
        child_abort.abort();
    }
    assert!(
        result.is_ok(),
        "a nonresponsive observer must be aborted after a bounded join"
    );
}

#[tokio::test]
async fn actor_sas_settlement_emits_exactly_one_terminal_and_clears_runtime() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let cred_dir = tempdir().expect("credential tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path())),
        data_dir.path(),
    );
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _) = broadcast::channel(16);
    let handle = AccountActor::spawn(
        store,
        action_tx,
        event_tx,
        LinkPreviewContext::default(),
        Arc::new(ComposerDraftLeaseRegistry::new()),
    );

    let cases = [
        SyntheticVerificationTerminal::Success,
        SyntheticVerificationTerminal::Cancelled(VerificationCancelReason::User),
        SyntheticVerificationTerminal::Cancelled(VerificationCancelReason::Mismatch),
        SyntheticVerificationTerminal::Failed(TrustOperationFailureKind::Timeout),
        SyntheticVerificationTerminal::Failed(TrustOperationFailureKind::Sdk),
    ];
    for (index, terminal) in cases.into_iter().enumerate() {
        let flow_id = index as u64 + 100;
        assert!(
            handle
                .send(AccountMessage::ConfigureSyntheticVerification { flow_id })
                .await
        );
        assert!(
            handle
                .send(AccountMessage::SettleSyntheticVerification { flow_id, terminal })
                .await
        );
        let actions = action_rx.recv().await.expect("one terminal action");
        assert_eq!(
            actions.len(),
            1,
            "flow {flow_id} must emit one terminal action"
        );
        let terminal_request_id = match (&terminal, actions.as_slice()) {
            (
                SyntheticVerificationTerminal::Success,
                [AppAction::VerificationCompleted { request_id }],
            )
            | (
                SyntheticVerificationTerminal::Cancelled(_),
                [AppAction::VerificationCancelled { request_id, .. }],
            )
            | (
                SyntheticVerificationTerminal::Failed(_),
                [AppAction::VerificationFailed { request_id, .. }],
            ) => *request_id,
            unexpected => panic!("unexpected terminal projection: {unexpected:?}"),
        };
        assert_eq!(terminal_request_id, flow_id);

        let (response, inspected) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::InspectVerificationRuntime { response })
                .await
        );
        assert_eq!(
            inspected.await.expect("runtime inspection"),
            (false, false, false, false, false, false, false)
        );

        assert!(
            handle
                .send(AccountMessage::SettleSyntheticVerification { flow_id, terminal })
                .await
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), action_rx.recv())
                .await
                .is_err(),
            "stale terminal duplicated flow {flow_id}"
        );
    }
    let settled_flow_ids = koushi_diagnostics::test_support::detail_snapshot().records
        [diagnostic_start..]
        .iter()
        .filter(|record| {
            record.event.source == "core.sas_verification" && record.event.stage == "settled"
        })
        .filter_map(|record| {
            record
                .event
                .fields
                .iter()
                .find_map(|field| (field.key == "flow_id").then_some(&field.value))
        })
        .filter_map(|value| match value {
            koushi_diagnostics::DiagnosticValue::Count(flow_id) => Some(*flow_id),
            _ => None,
        })
        .filter(|flow_id| (100..=104).contains(flow_id))
        .collect::<Vec<_>>();
    assert_eq!(settled_flow_ids, vec![100, 101, 102, 103, 104]);
    shutdown_and_ack(&handle).await;
}

#[test]
fn sas_verification_tokens_are_closed_and_private_safe() {
    use koushi_sdk::MatrixSasState as SasState;
    use koushi_sdk::MatrixVerificationCancelKind as CancelKind;
    use koushi_sdk::MatrixVerificationRequestState as RequestState;

    assert_eq!(
        verification_request_state_token(&RequestState::Created),
        "created"
    );
    assert_eq!(
        verification_request_state_token(&RequestState::Requested),
        "requested"
    );
    assert_eq!(
        verification_request_state_token(&RequestState::Ready),
        "ready"
    );
    assert_eq!(
        verification_request_state_token(&RequestState::Done),
        "done"
    );
    assert_eq!(
        verification_request_state_token(&RequestState::Cancelled {
            kind: CancelKind::Timeout,
            cancelled_by_us: false,
        }),
        "cancelled"
    );
    assert_eq!(
        verification_request_state_token(&RequestState::UnsupportedMethod),
        "unsupported_method"
    );

    let cancel_kinds = [
        (CancelKind::UnknownMethod, "unknown_method"),
        (CancelKind::KeyMismatch, "key_mismatch"),
        (CancelKind::User, "user"),
        (CancelKind::Timeout, "timeout"),
        (CancelKind::AcceptedElsewhere, "accepted_elsewhere"),
        (CancelKind::Other, "other"),
    ];
    for (kind, token) in cancel_kinds {
        assert_eq!(verification_cancel_kind_token(kind), token);
    }

    let sas_states = [
        (SasState::Created, "created"),
        (SasState::Started, "started"),
        (SasState::Accepted, "accepted"),
        (
            SasState::SasPresented { emojis: Vec::new() },
            "sas_presented",
        ),
        (SasState::Confirmed, "confirmed"),
        (SasState::Done, "done"),
        (
            SasState::Cancelled {
                kind: CancelKind::Timeout,
                cancelled_by_us: false,
            },
            "cancelled",
        ),
        (SasState::UnsupportedShortAuth, "unsupported_short_auth"),
    ];
    for (state, token) in sas_states {
        assert_eq!(sas_state_token(&state), token);
    }

    let failure_kinds = [
        (TrustOperationFailureKind::Cancelled, "cancelled"),
        (TrustOperationFailureKind::Mismatch, "mismatch"),
        (
            TrustOperationFailureKind::InvalidPassphrase,
            "invalid_passphrase",
        ),
        (TrustOperationFailureKind::Network, "network"),
        (TrustOperationFailureKind::Forbidden, "forbidden"),
        (TrustOperationFailureKind::Timeout, "timeout"),
        (TrustOperationFailureKind::Sdk, "sdk"),
    ];
    for (kind, token) in failure_kinds {
        assert_eq!(trust_failure_token(kind), token);
    }
    let recovery_failure_kinds = [
        (
            RecoveryFailureKind::InvalidRecoveryKey,
            "invalid_recovery_key",
        ),
        (RecoveryFailureKind::Network, "network"),
        (RecoveryFailureKind::Server, "server"),
        (RecoveryFailureKind::Timeout, "timeout"),
    ];
    for (kind, token) in recovery_failure_kinds {
        assert_eq!(recovery_failure_token(kind), token);
    }

    let wait_states = [
        (
            SasVerificationWaitState::RecipientDevices,
            "recipient_devices",
        ),
        (
            SasVerificationWaitState::ToDeviceDelivery,
            "to_device_delivery",
        ),
        (SasVerificationWaitState::RemoteAccept, "remote_accept"),
        (SasVerificationWaitState::SasStart, "sas_start"),
        (SasVerificationWaitState::Mac, "mac"),
        (
            SasVerificationWaitState::CrossSigningSettlement,
            "cross_signing_settlement",
        ),
        (
            SasVerificationWaitState::NormalSyncResume,
            "normal_sync_resume",
        ),
    ];
    for (state, token) in wait_states {
        assert_eq!(sas_waiting_for_token(state), token);
    }

    assert_eq!(
        verification_terminal_token(VerificationTerminal::Success),
        "success"
    );
    assert_eq!(
        verification_terminal_token(VerificationTerminal::Cancelled(
            VerificationCancelReason::User,
        )),
        "cancelled"
    );
    assert_eq!(
        verification_terminal_token(VerificationTerminal::Failed(
            TrustOperationFailureKind::Timeout,
        )),
        "failed"
    );
}

#[test]
fn sas_cancel_diagnostic_contains_only_closed_private_safe_fields() {
    use koushi_sdk::MatrixSasState as SasState;
    use koushi_sdk::MatrixVerificationCancelKind as CancelKind;

    let cancelled = sas_state_changed_event(
        41,
        &SasState::Cancelled {
            kind: CancelKind::Timeout,
            cancelled_by_us: false,
        },
    );
    assert_eq!(
        koushi_diagnostics::format_event(&cancelled),
        "stage=sas_state_changed flow_id=41 state=cancelled cancel_kind=timeout cancelled_by_us=false"
    );

    let accepted = sas_state_changed_event(42, &SasState::Accepted);
    assert_eq!(
        koushi_diagnostics::format_event(&accepted),
        "stage=sas_state_changed flow_id=42 state=accepted waiting_for=sas_start"
    );

    let settled = sas_settled_event(
        43,
        VerificationTerminal::Failed(TrustOperationFailureKind::Timeout),
        Some(SasVerificationWaitState::RemoteAccept),
    );
    assert_eq!(
        koushi_diagnostics::format_event(&settled),
        "stage=settled flow_id=43 terminal=failed waiting_for=remote_accept failure_kind=timeout"
    );

    let timeout = sas_timeout_fired_event(44, Some(SasVerificationWaitState::Mac));
    assert_eq!(
        koushi_diagnostics::format_event(&timeout),
        "stage=timeout_fired flow_id=44 waiting_for=mac"
    );

    let recovery = recovery_verification_event("settled", 45)
        .field(DiagnosticField::token("terminal", "failed"))
        .field(DiagnosticField::token(
            "failure_kind",
            recovery_failure_token(RecoveryFailureKind::InvalidRecoveryKey),
        ));
    assert_eq!(
        koushi_diagnostics::format_event(&recovery),
        "stage=settled flow_id=45 flow_type=recovery_key terminal=failed failure_kind=invalid_recovery_key"
    );
}

#[tokio::test]
async fn own_user_sas_start_helper_traces_started_pending_and_failed_results() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();

    assert_eq!(
        run_own_user_sas_start(211, "request_ready", async {
            Ok::<_, koushi_sdk::E2eeTrustError>(Some(7_u8))
        })
        .await
        .expect("started result"),
        Some(7)
    );
    assert_eq!(
        run_own_user_sas_start(212, "initial", async {
            Ok::<Option<u8>, koushi_sdk::E2eeTrustError>(None)
        })
        .await
        .expect("pending result"),
        None
    );
    assert!(
        run_own_user_sas_start(213, "provisional_encryption_sync", async {
            Err::<Option<u8>, _>(koushi_sdk::E2eeTrustError::Sdk(
                "private SDK detail".to_owned(),
            ))
        })
        .await
        .is_err()
    );

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let events = records[diagnostic_start..]
        .iter()
        .filter(|record| record.event.source == "core.sas_verification")
        .map(|record| koushi_diagnostics::format_event(&record.event))
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        vec![
            "stage=sas_start_attempted flow_id=211 source=request_ready",
            "stage=sas_start_finished flow_id=211 source=request_ready outcome=started",
            "stage=sas_start_attempted flow_id=212 source=initial",
            "stage=sas_start_finished flow_id=212 source=initial outcome=pending",
            "stage=sas_start_attempted flow_id=213 source=provisional_encryption_sync",
            "stage=sas_start_finished flow_id=213 source=provisional_encryption_sync outcome=failed failure_kind=sdk",
        ]
    );
    assert!(!events.join(" ").contains("private SDK detail"));
}

#[test]
fn sas_verification_diagnostic_records_without_stderr() {
    let output = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .args([
        "--exact",
        "account::verification::tests::sas_verification_diagnostic_child",
        "--ignored",
        "--nocapture",
    ])
    .output()
    .expect("SAS verification diagnostic child should run");
    assert!(output.status.success(), "child failed: {output:?}");

    let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
    assert!(
        stderr.is_empty(),
        "private diagnostics stay in the buffer only"
    );

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
            record["event"]["source"] == "core.sas_verification"
                && record["event"]["stage"] == "request_state_changed"
        })
    }));
}

#[test]
#[ignore]
fn sas_verification_diagnostic_child() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_sas_verification_event(
        sas_verification_event("request_state_changed", 41)
            .field(DiagnosticField::token("state", "cancelled"))
            .field(DiagnosticField::token("cancel_kind", "timeout"))
            .field(DiagnosticField::boolean("cancelled_by_us", false)),
    );
    println!(
        "{}",
        serde_json::to_string(&koushi_diagnostics::test_support::detail_snapshot())
            .expect("diagnostic snapshot should serialize")
    );
}

#[test]
fn incoming_verification_flow_ids_use_reserved_internal_namespace() {
    let request_id = incoming_verification_request_id(INCOMING_VERIFICATION_FLOW_ID_BASE);

    assert_eq!(request_id.connection_id, RuntimeConnectionId(0));
    assert_eq!(request_id.sequence, INCOMING_VERIFICATION_FLOW_ID_BASE);
}

#[tokio::test]
async fn own_user_sas_proof_success_enters_shared_authoritative_promotion_path() {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    let flow_id = 83;
    handle
        .send(AccountMessage::ConfigureSyntheticVerification { flow_id })
        .await;
    handle
        .send(AccountMessage::SettleSyntheticVerification {
            flow_id,
            terminal: SyntheticVerificationTerminal::Success,
        })
        .await;
    let mut verification_completed = false;
    let mut authoritative_recheck_settled = false;
    while !(verification_completed && authoritative_recheck_settled) {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        verification_completed |= matches!(
            actions.as_slice(),
            [AppAction::VerificationCompleted { request_id }] if *request_id == flow_id
        );
        authoritative_recheck_settled |= matches!(
            actions.as_slice(),
            [AppAction::AuthoritativeDeviceTrustChanged {
                trust: koushi_state::CurrentDeviceTrustState::Unknown
                    | koushi_state::CurrentDeviceTrustState::Unverified,
                ..
            }]
        );
    }
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
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
async fn identity_reset_auth_without_session_settles_pending_state() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    let flow_id = 99;
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::SubmitIdentityResetAuth {
                    request_id,
                    flow_id,
                    request: koushi_state::IdentityResetAuthRequest::OAuthApproved,
                }
            ))
            .await
    );

    let actions = action_rx.recv().await.expect("trust failure action batch");
    assert_eq!(
        actions,
        vec![AppAction::ResetIdentityFailed {
            request_id: flow_id,
            kind: koushi_state::TrustOperationFailureKind::Sdk,
        }]
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
    }
}

#[test]
fn verification_protection_summary_is_private_data_free() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let counters = koushi_sdk::IncomingVerificationRequestProtectionCounters {
        unknown_sender_deferred: 2,
        key_query_replays: 1,
        released_deliveries: 3,
        suppressed_sas_start_replays: 4,
    };
    record_incoming_verification_protection_summary(
        &counters,
        VERIFICATION_PROTECTION_SUMMARY_TRIGGER_RESTORE,
    );

    let snapshot = koushi_diagnostics::snapshot();
    let summary = snapshot
        .records
        .iter()
        .find(|record| record.event.source == "core.verification_protection_summary")
        .expect("summary recorded");
    let text = format!("{:?}", summary.event);
    for private in [
        "@",
        "!",
        "room_id",
        "user_id",
        "device_id",
        "event_id",
        "session_id",
        "http",
    ] {
        assert!(!text.contains(private), "{private} leaked into summary: {text}");
    }
    assert!(text.contains("unknown_sender_deferred"));
    assert!(text.contains("suppressed_sas_start_replays"));
}
