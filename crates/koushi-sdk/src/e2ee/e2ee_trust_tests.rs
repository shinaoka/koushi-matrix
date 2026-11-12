use super::{
    E2eeTrustError, IdentityFact, KeyBackupRestoreScope, KeyBackupRestoreSummary,
    MatrixCrossSigningStatus, MatrixIdentityResetAuthType, MatrixIncomingVerificationRequest,
    MatrixIncomingVerificationRequestObserver, PersistableMatrixSession, RecoveryFact,
    RoomKeyExportSummary, RoomKeyImportSummary, SecureBackupSetupSummary, VerificationMethodFacts,
    accept_sas_verification, accept_verification_request, bootstrap_cross_signing,
    bootstrap_secure_backup, cancel_sas_verification, cancel_verification_request,
    change_secure_backup_passphrase, complete_identity_reset, confirm_sas_verification,
    cross_signing_status, enable_key_backup, export_room_keys_to_file,
    forward_incoming_verification_deliveries, import_room_keys_from_file,
    map_backup_state_to_desktop, map_cross_signing_status_to_desktop,
    map_identity_reset_auth_type_to_desktop, map_sdk_sas_emojis_to_desktop,
    map_sdk_verification_state, map_verification_method_facts, mismatch_sas_verification,
    observe_incoming_verification_requests, request_device_verification, reset_identity,
    restore_key_backup, restore_session, start_sas_verification, write_recovery_key_material,
};
use futures_util::stream;
use koushi_state::{
    AuthSecret, CrossSigningStatus, CurrentDeviceTrustState, IdentityResetAuthType,
    KeyBackupStatus, SasEmoji, SessionInfo, VerificationAccountKind, VerificationMethodCapability,
};
use matrix_sdk::encryption::backups::BackupState;
use matrix_sdk::{
    ruma::{owned_device_id, owned_user_id},
    test_utils::mocks::MatrixMockServer,
};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
const MATRIX_KEY_EXPORT_HEADER: &str = "-----BEGIN MEGOLM SESSION DATA-----";
const MATRIX_KEY_EXPORT_FOOTER: &str = "-----END MEGOLM SESSION DATA-----";
struct FakeIncomingDelivery {
    id: u8,
    product: Option<u8>,
    committed: bool,
    commits: Arc<Mutex<Vec<u8>>>,
    uncommitted_drops: Arc<Mutex<Vec<u8>>>,
}
impl Drop for FakeIncomingDelivery {
    fn drop(&mut self) {
        if !self.committed {
            self.uncommitted_drops.lock().unwrap().push(self.id);
        }
    }
}
pub(super) const ELEMENT_COMPATIBLE_KEY_EXPORT: &str = "\
-----BEGIN MEGOLM SESSION DATA-----\n\
Af7mGhlzQ+eGvHu93u0YXd3D/+vYMs3E7gQqOhuCtkvGAAAAASH7pEdWvFyAP1JUisAcpEo\n\
Xke2Q7Kr9hVl/SCc6jXBNeJCZcrUbUV4D/tRQIl3E9L4fOk928YI1J+3z96qiH0uE7hpsCI\n\
CkHKwjPU+0XTzFdIk1X8H7sZ+MD/2Sg/q3y8rtUjz7uEj4GUTnb+9SCOTVmJsRfqgUpM1CU\n\
bDLytHf1JkohY4tWEgpsCc67xdzgodjr12qYrfg/zNm3LGpxlrffJknw4rk5QFTj4kMbqbD\n\
ZZgDTni+HxRTDGge2J620lMOiznvXX+H09Rwruqx5aJvvaaKd86jWRpiO2oSFqHn4u5ONl9\n\
41uzm62Sj0eIm6ZbA9NQs87jQw4LxsejhZVL+NdjIg80zVSBTWhTdo0DTnbFSNP4ReOiz0U\n\
XosOF8A5T8Vdx2nvA0GXltfcHKVKQYh/LJAkNQ7P9UYL4ae/5TtQZkhB1KxCLTRWqADCl53\n\
uBMGpG53EMgY6G6K2DEIOkcv7sdXQF5WpemiSWZqJRWj+cjfs9BpCTbkp/rszWFl2TniWpR\n\
RqIbT2jORlN4rTvdtF0F4z1pqP4qWyR3sLNTkXm9CFRzWADNG0RDZKxbCoo6RPvtaCTfaHo\n\
SwfvzBS6CjfAG+FOugpV48o7+XetaUUPZ6/tZSPhCdeV8eP9q5r0QwWeXFogzoNzWt4HYx9\n\
MdXxzD+f0mtg5gzehrrEEARwI2bCvPpHxlt/Na9oW/GBpkjwR1LSKgg4CtpRyWngPjdEKpZ\n\
GYW19pdjg0qdXNk/eqZsQTsNWVo6A\n\
-----END MEGOLM SESSION DATA-----";
#[tokio::test]
async fn incoming_verification_observer_shutdown_joins_typed_delivery_task() {
    let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
    )
    .expect("synthetic session should deserialize");
    let session = restore_session(&persistable)
        .await
        .expect("synthetic session should restore");
    let mut observer = observe_incoming_verification_requests(&session).await;
    let abort_handle = observer
        .incoming_request_task
        .as_ref()
        .expect("a restored session has a typed incoming-request subscription")
        .abort_handle();

    observer.shutdown().await;

    assert!(abort_handle.is_finished());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_incoming_observer_shutdown_retains_inner_task_ownership() {
    struct TaskAlive(Arc<AtomicBool>);
    impl Drop for TaskAlive {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
    )
    .expect("synthetic session should deserialize");
    let session = restore_session(&persistable)
        .await
        .expect("synthetic session should restore");
    let mut observer = observe_incoming_verification_requests(&session).await;
    let original = observer
        .incoming_request_task
        .take()
        .expect("a restored session has a typed incoming-request subscription");
    original.abort();
    let _ = original.await;

    let alive = Arc::new(AtomicBool::new(true));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    observer.incoming_request_task = Some(tokio::spawn({
        let alive = Arc::clone(&alive);
        async move {
            let _alive = TaskAlive(alive);
            let _ = started_tx.send(());
            let _ = release_rx.recv();
            std::future::pending::<()>().await;
        }
    }));
    started_rx.await.expect("noncooperative inner task started");

    assert!(
        tokio::time::timeout(Duration::from_millis(20), observer.shutdown())
            .await
            .is_err(),
        "the fixture must cancel shutdown while the inner task cannot settle"
    );
    assert!(
        observer.incoming_request_task.is_some(),
        "shutdown cancellation must leave the JoinHandle with its observer owner"
    );
    assert!(alive.load(Ordering::SeqCst));

    release_tx
        .send(())
        .expect("release noncooperative inner task");
    observer.shutdown().await;
    assert!(!alive.load(Ordering::SeqCst));
}
#[tokio::test]
async fn terminal_incoming_head_is_committed_before_actionable_tail() {
    let commits = Arc::new(Mutex::new(Vec::new()));
    let uncommitted_drops = Arc::new(Mutex::new(Vec::new()));
    let delivery = |id, product| FakeIncomingDelivery {
        id,
        product,
        committed: false,
        commits: commits.clone(),
        uncommitted_drops: uncommitted_drops.clone(),
    };
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);

    forward_incoming_verification_deliveries(
        stream::iter([delivery(1, None), delivery(2, Some(42))]),
        sender,
        |delivery| delivery.product,
        |mut delivery| {
            delivery.committed = true;
            delivery.commits.lock().unwrap().push(delivery.id);
        },
    )
    .await;

    assert_eq!(receiver.recv().await, Some(42));
    assert_eq!(*commits.lock().unwrap(), vec![1, 2]);
    assert!(uncommitted_drops.lock().unwrap().is_empty());
}
#[tokio::test]
async fn actionable_incoming_delivery_commits_only_after_product_send_success() {
    let commits = Arc::new(Mutex::new(Vec::new()));
    let uncommitted_drops = Arc::new(Mutex::new(Vec::new()));
    let delivery = FakeIncomingDelivery {
        id: 1,
        product: Some(42),
        committed: false,
        commits: commits.clone(),
        uncommitted_drops: uncommitted_drops.clone(),
    };
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);

    forward_incoming_verification_deliveries(
        stream::iter([delivery]),
        sender,
        |delivery| delivery.product,
        |mut delivery| {
            delivery.committed = true;
            delivery.commits.lock().unwrap().push(delivery.id);
        },
    )
    .await;

    assert!(commits.lock().unwrap().is_empty());
    assert_eq!(*uncommitted_drops.lock().unwrap(), vec![1]);
}
#[tokio::test]
async fn verification_raw_redelivery_reuses_the_same_product_flow_identity() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;

    let alice_user_id = owned_user_id!("@alice:example.org");
    let alice_device_id = owned_device_id!("ALICEDEVICE");
    let alice = server
        .client_builder_for_crypto_end_to_end(&alice_user_id, &alice_device_id)
        .build()
        .await;
    let bob_user_id = owned_user_id!("@bob:example.org");
    let bob_device_id = owned_device_id!("BOBDEVICE");
    let bob = server
        .client_builder_for_crypto_end_to_end(&bob_user_id, &bob_device_id)
        .build()
        .await;

    // Publish Bob's device keys without teaching Alice about Bob. The first
    // request delivery must therefore use the passive unknown-sender
    // recovery path after its key query completes.
    server.mock_sync().ok_and_run(&bob, |_| {}).await;
    let session = super::MatrixClientSession {
        client: alice.clone(),
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
        info: SessionInfo {
            homeserver: server.server().uri(),
            user_id: alice_user_id.to_string(),
            device_id: alice_device_id.to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    };
    let mut observer = observe_incoming_verification_requests(&session).await;
    let request_timestamp = matrix_sdk::ruma::MilliSecondsSinceUnixEpoch::now().get();
    let request = json!({
            "sender": bob_user_id,
            "type": "m.key.verification.request",
            "content": {
                "from_device": bob_device_id,
                "transaction_id": "sender-key-recovery-flow",
                "methods": ["m.sas.v1"],
                "timestamp": request_timestamp,
        },
    });

    server
        .mock_sync()
        .ok_and_run(&alice, |builder| {
            builder.add_to_device_event(request.clone());
        })
        .await;
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), observer.recv())
        .await
        .expect("typed sender-key recovery should publish without polling delay")
        .expect("typed sender-key recovery should yield the request");
    assert_eq!(first.handle().flow_id(), "sender-key-recovery-flow");

    server
        .mock_sync()
        .ok_and_run(&alice, |builder| {
            builder.add_to_device_event(request);
        })
        .await;
    let repeated = tokio::time::timeout(std::time::Duration::from_secs(5), observer.recv())
        .await
        .expect("at-least-once transport should forward raw redelivery without polling delay")
        .expect("raw redelivery should remain observable");
    assert_eq!(repeated.handle().flow_id(), first.handle().flow_id());
}
#[test]
fn cross_signing_status_maps_to_private_data_free_desktop_status() {
    assert_eq!(
        map_cross_signing_status_to_desktop(None),
        CrossSigningStatus::Missing
    );
    assert_eq!(
        map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
            has_master: true,
            has_self_signing: true,
            has_user_signing: true,
        })),
        CrossSigningStatus::Trusted
    );
    assert_eq!(
        map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
            has_master: true,
            has_self_signing: false,
            has_user_signing: true,
        })),
        CrossSigningStatus::NotTrusted
    );
}
#[test]
fn current_device_trust_maps_all_sdk_verification_states() {
    use matrix_sdk::encryption::VerificationState;

    assert_eq!(
        map_sdk_verification_state(VerificationState::Unknown),
        CurrentDeviceTrustState::Unknown
    );
    assert_eq!(
        map_sdk_verification_state(VerificationState::Verified),
        CurrentDeviceTrustState::Verified
    );
    assert_eq!(
        map_sdk_verification_state(VerificationState::Unverified),
        CurrentDeviceTrustState::Unverified
    );
}
#[test]
fn verification_method_discovery_distinguishes_identity_facts() {
    let existing_with_sas = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Existing,
        verified_other_device_count: 2,
        recovery: RecoveryFact::Unavailable,
    });
    assert_eq!(
        existing_with_sas.account_kind,
        VerificationAccountKind::ExistingIdentity
    );
    assert_eq!(
        existing_with_sas.methods,
        vec![VerificationMethodCapability::ExistingDeviceSas]
    );

    let new_identity = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Missing,
        verified_other_device_count: 0,
        recovery: RecoveryFact::Unavailable,
    });
    assert_eq!(
        new_identity.account_kind,
        VerificationAccountKind::NewIdentity
    );
    assert_eq!(
        new_identity.methods,
        vec![VerificationMethodCapability::Bootstrap]
    );

    let unknown = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Unknown,
        verified_other_device_count: 0,
        recovery: RecoveryFact::Available,
    });
    assert_eq!(unknown.account_kind, VerificationAccountKind::Unknown);
    assert!(unknown.methods.is_empty());

    let existing_with_recovery = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Existing,
        verified_other_device_count: 0,
        recovery: RecoveryFact::Available,
    });
    assert_eq!(
        existing_with_recovery.methods,
        vec![
            VerificationMethodCapability::RecoveryKey,
            VerificationMethodCapability::SecurityPhrase,
        ]
    );

    let existing_without_proof = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Existing,
        verified_other_device_count: 0,
        recovery: RecoveryFact::Unavailable,
    });
    assert_eq!(
        existing_without_proof.account_kind,
        VerificationAccountKind::ExistingIdentity
    );
    assert!(existing_without_proof.methods.is_empty());

    let sas_survives_unknown_recovery = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Existing,
        verified_other_device_count: 1,
        recovery: RecoveryFact::Unknown,
    });
    assert_eq!(
        sas_survives_unknown_recovery.methods,
        vec![VerificationMethodCapability::ExistingDeviceSas]
    );

    let unknown_without_known_proof = map_verification_method_facts(VerificationMethodFacts {
        identity: IdentityFact::Existing,
        verified_other_device_count: 0,
        recovery: RecoveryFact::Unknown,
    });
    assert_eq!(
        unknown_without_known_proof.account_kind,
        VerificationAccountKind::Unknown
    );
}
#[test]
fn own_user_proof_eligibility_requires_distinct_owner_signed_unblocked_device() {
    assert!(super::is_eligible_own_user_proof_device(
        "CURRENT", "OTHER", true, false
    ));
    assert!(!super::is_eligible_own_user_proof_device(
        "CURRENT", "CURRENT", true, false
    ));
    assert!(!super::is_eligible_own_user_proof_device(
        "CURRENT", "OTHER", false, false
    ));
    assert!(!super::is_eligible_own_user_proof_device(
        "CURRENT", "OTHER", true, true
    ));
}
#[test]
fn own_user_request_recipient_requires_a_distinct_owner_signed_device() {
    assert!(super::is_own_user_verification_recipient(
        "CURRENT", "OTHER", true
    ));
    assert!(!super::is_own_user_verification_recipient(
        "CURRENT", "CURRENT", true
    ));
    assert!(!super::is_own_user_verification_recipient(
        "CURRENT", "OTHER", false
    ));
}
#[test]
fn own_user_sas_recipient_diagnostics_distinguish_sender_and_interactive_targets() {
    use super::OwnUserSasDeviceFact as Fact;

    let diagnostics = super::own_user_sas_recipient_diagnostics([
        Fact {
            is_current: true,
            cross_signed_by_owner: false,
            blocked: false,
            dehydrated: false,
            curve_key_present: true,
            ed25519_key_present: true,
        },
        Fact {
            is_current: false,
            cross_signed_by_owner: true,
            blocked: false,
            dehydrated: false,
            curve_key_present: true,
            ed25519_key_present: true,
        },
        Fact {
            is_current: false,
            cross_signed_by_owner: true,
            blocked: false,
            dehydrated: true,
            curve_key_present: true,
            ed25519_key_present: true,
        },
        Fact {
            is_current: false,
            cross_signed_by_owner: true,
            blocked: true,
            dehydrated: false,
            curve_key_present: false,
            ed25519_key_present: true,
        },
        Fact {
            is_current: false,
            cross_signed_by_owner: false,
            blocked: false,
            dehydrated: false,
            curve_key_present: true,
            ed25519_key_present: true,
        },
    ]);

    assert!(diagnostics.sender_device_query_visible);
    assert!(diagnostics.sender_curve_key_present);
    assert!(diagnostics.sender_ed25519_key_present);
    assert_eq!(diagnostics.other_device_count, 4);
    assert_eq!(diagnostics.recipient_count, 3);
    assert_eq!(diagnostics.eligible_device_count, 2);
    assert_eq!(diagnostics.interactive_recipient_count, 1);
    assert_eq!(diagnostics.dehydrated_recipient_count, 1);
}
#[test]
fn sas_delivery_event_contains_only_closed_private_safe_fields() {
    let event = super::sas_delivery_event("recipients_resolved", 41)
        .field(koushi_diagnostics::DiagnosticField::count(
            "other_device_count",
            3,
        ))
        .field(koushi_diagnostics::DiagnosticField::count(
            "recipient_count",
            1,
        ));
    assert_eq!(event.source, "core.sas_verification");
    assert_eq!(
        koushi_diagnostics::format_event(&event),
        "stage=recipients_resolved flow_id=41 other_device_count=3 recipient_count=1"
    );
}
#[test]
fn sas_delivery_waiting_event_identifies_private_safe_wait_state() {
    let event = super::sas_delivery_waiting_event(43, "to_device_delivery");

    assert_eq!(
        koushi_diagnostics::format_event(&event),
        "stage=waiting flow_id=43 waiting_for=to_device_delivery"
    );
}
#[test]
fn sas_recipients_resolved_event_includes_sender_readiness_without_identifiers() {
    let event = super::sas_recipients_resolved_event(
        42,
        super::OwnUserSasRecipientDiagnostics {
            other_device_count: 9,
            recipient_count: 6,
            eligible_device_count: 6,
            sender_device_query_visible: true,
            sender_curve_key_present: true,
            sender_ed25519_key_present: true,
            interactive_recipient_count: 5,
            dehydrated_recipient_count: 1,
        },
    );

    assert_eq!(
        koushi_diagnostics::format_event(&event),
        "stage=recipients_resolved flow_id=42 other_device_count=9 recipient_count=6 eligible_device_count=6 sender_device_query_visible=true sender_curve_key_present=true sender_ed25519_key_present=true interactive_recipient_count=5 dehydrated_recipient_count=1"
    );
}
#[test]
fn verification_cancel_codes_map_to_closed_private_safe_categories() {
    use super::MatrixVerificationCancelKind as Kind;

    assert_eq!(
        super::map_verification_cancel_kind("m.unknown_method"),
        Kind::UnknownMethod
    );
    assert_eq!(
        super::map_verification_cancel_kind("m.key_mismatch"),
        Kind::KeyMismatch
    );
    assert_eq!(super::map_verification_cancel_kind("m.user"), Kind::User);
    assert_eq!(
        super::map_verification_cancel_kind("m.timeout"),
        Kind::Timeout
    );
    assert_eq!(
        super::map_verification_cancel_kind("m.accepted"),
        Kind::AcceptedElsewhere
    );
    assert_eq!(
        super::map_verification_cancel_kind("m.future_code"),
        Kind::Other
    );
}
#[test]
fn sas_cancellation_maps_to_closed_private_safe_projection() {
    use super::{MatrixSasState as SasState, MatrixVerificationCancelKind as CancelKind};

    let state = super::map_sas_cancellation("m.timeout", false);

    assert_eq!(
        state,
        SasState::Cancelled {
            kind: CancelKind::Timeout,
            cancelled_by_us: false,
        }
    );
    let debug = format!("{state:?}");
    assert_eq!(debug, "Cancelled { kind: Timeout, cancelled_by_us: false }");
    assert!(!debug.contains("m.timeout"));

    let unknown = super::map_sas_cancellation("m.future_private_code", true);
    assert_eq!(
        unknown,
        SasState::Cancelled {
            kind: CancelKind::Other,
            cancelled_by_us: true,
        }
    );
    assert!(!format!("{unknown:?}").contains("future_private_code"));
}
#[test]
fn own_user_sas_api_returns_only_an_opaque_adapter_handle() {
    let _ = super::request_own_user_sas_verification;
    let _opaque: Option<super::MatrixOwnUserVerificationHandle> = None;
    assert!(!std::any::type_name::<super::MatrixOwnUserVerificationHandle>().contains('@'));
}
#[test]
fn key_backup_state_maps_to_private_data_free_desktop_status() {
    assert_eq!(
        map_backup_state_to_desktop(BackupState::Unknown),
        KeyBackupStatus::Unknown
    );
    assert_eq!(
        map_backup_state_to_desktop(BackupState::Enabled),
        KeyBackupStatus::Enabled {
            version: "available".to_owned(),
        }
    );
    assert_eq!(
        map_backup_state_to_desktop(BackupState::Downloading),
        KeyBackupStatus::Restoring {
            request_id: 0,
            version: None,
            restored_rooms: 0,
            total_rooms: None,
        }
    );
}
#[test]
fn e2ee_trust_error_debug_redacts_sdk_details() {
    let error = E2eeTrustError::Sdk("raw matrix sdk error with @alice:example.test".to_owned());
    let debug = format!("{error:?}");

    assert!(!debug.contains("@alice:example.test"));
    assert!(!debug.contains("raw matrix sdk error"));
    assert!(debug.contains("Sdk"));
}
#[test]
fn key_backup_restore_summary_declares_joined_room_scope() {
    let summary = KeyBackupRestoreSummary {
        scope: KeyBackupRestoreScope::JoinedRooms,
        version: Some("available".to_owned()),
        restored_rooms: 2,
        total_rooms: Some(3),
    };

    let debug = format!("{summary:?}");
    assert!(debug.contains("JoinedRooms"));
    assert!(!debug.contains("BackupWide"));
    assert!(!debug.contains("AllRooms"));
}
#[test]
fn room_key_file_transfer_summaries_are_private_data_free() {
    let export_summary = RoomKeyExportSummary {
        exported_sessions: None,
    };
    let import_summary = RoomKeyImportSummary {
        imported_count: 1,
        total_count: 1,
    };

    assert_eq!(export_summary.exported_sessions, None);
    assert_eq!(import_summary.imported_count, 1);
    assert_eq!(import_summary.total_count, 1);
    assert!(!format!("{export_summary:?}").contains("MEGOLM"));
    assert!(!format!("{import_summary:?}").contains("MEGOLM"));
}
#[test]
fn secure_backup_setup_summary_is_private_data_free() {
    let summary = SecureBackupSetupSummary {
        recovery_key: zeroize::Zeroizing::new("synthetic-summary-key".to_owned()),
    };

    let debug = format!("{summary:?}");
    assert!(debug.contains("recovery_key"));
    assert!(!debug.contains("synthetic-summary-key"), "{debug}");
}
#[test]
fn recovery_key_delivery_writes_native_artifact_without_debugging_material() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("recovery-artifact.txt");
    let artifact_payload = String::from("fixture-artifact-material");

    write_recovery_key_material(&artifact_payload, path.clone())
        .expect("artifact write should succeed");

    assert_eq!(
        std::fs::read_to_string(&path).expect("read artifact"),
        artifact_payload
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(path)
                .expect("artifact metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn recovery_key_delivery_refuses_to_overwrite_an_existing_artifact() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("existing-artifact.txt");
    std::fs::write(&path, "keep-me").expect("write existing artifact");

    let error = write_recovery_key_material("fixture-artifact-material", path.clone())
        .expect_err("existing artifact must not be overwritten");

    assert_eq!(error, E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed);
    assert_eq!(
        std::fs::read_to_string(path).expect("read artifact"),
        "keep-me"
    );
}
#[tokio::test]
async fn room_key_import_accepts_element_compatible_key_export_envelope() {
    assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.starts_with(MATRIX_KEY_EXPORT_HEADER));
    assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.ends_with(MATRIX_KEY_EXPORT_FOOTER));

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("element-compatible-room-keys.txt");
    std::fs::write(&path, ELEMENT_COMPATIBLE_KEY_EXPORT).expect("write fixture");
    let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
    )
    .expect("synthetic session should deserialize");
    let session = restore_session(&persistable)
        .await
        .expect("synthetic session should restore");

    let summary = import_room_keys_from_file(&session, path, &AuthSecret::new("1234"))
        .await
        .expect("Matrix/Element key export envelope should import");

    assert_eq!(summary.total_count, 1);
}
#[test]
fn e2ee_trust_public_async_api_is_exposed() {
    let _ = cross_signing_status;
    let _ = bootstrap_cross_signing;
    let _ = enable_key_backup;
    let _ = restore_key_backup;
    let _ = reset_identity;
    let _ = complete_identity_reset;
    let _ = request_device_verification;
    let _ = accept_verification_request;
    let _ = start_sas_verification;
    let _ = accept_sas_verification;
    let _ = confirm_sas_verification;
    let _ = mismatch_sas_verification;
    let _ = cancel_verification_request;
    let _ = cancel_sas_verification;
    let _ = observe_incoming_verification_requests;
    let _ = export_room_keys_to_file;
    let _ = import_room_keys_from_file;
    let _ = bootstrap_secure_backup;
    let _ = change_secure_backup_passphrase;
    let _: Option<MatrixIncomingVerificationRequest> = None;
    let _: Option<MatrixIncomingVerificationRequestObserver> = None;
}
#[test]
fn sas_emojis_map_to_desktop_dto_without_sdk_types() {
    let emojis = [
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🐶",
            description: "Dog",
        },
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🐱",
            description: "Cat",
        },
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🦁",
            description: "Lion",
        },
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🐎",
            description: "Horse",
        },
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🦄",
            description: "Unicorn",
        },
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🐷",
            description: "Pig",
        },
        matrix_sdk::encryption::verification::Emoji {
            symbol: "🐘",
            description: "Elephant",
        },
    ];

    assert_eq!(
        map_sdk_sas_emojis_to_desktop(emojis),
        vec![
            SasEmoji {
                symbol: "🐶".to_owned(),
                description: "Dog".to_owned(),
            },
            SasEmoji {
                symbol: "🐱".to_owned(),
                description: "Cat".to_owned(),
            },
            SasEmoji {
                symbol: "🦁".to_owned(),
                description: "Lion".to_owned(),
            },
            SasEmoji {
                symbol: "🐎".to_owned(),
                description: "Horse".to_owned(),
            },
            SasEmoji {
                symbol: "🦄".to_owned(),
                description: "Unicorn".to_owned(),
            },
            SasEmoji {
                symbol: "🐷".to_owned(),
                description: "Pig".to_owned(),
            },
            SasEmoji {
                symbol: "🐘".to_owned(),
                description: "Elephant".to_owned(),
            },
        ]
    );
}
#[test]
fn identity_reset_auth_type_maps_to_private_data_free_desktop_status() {
    assert_eq!(
        map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::Uiaa),
        IdentityResetAuthType::Uiaa
    );
    assert_eq!(
        map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::OAuth),
        IdentityResetAuthType::OAuth
    );
}
