use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use koushi_key::StoredMatrixSession;
use koushi_protocol::SessionKeyId;

use koushi_state::{
    AppAction, DeviceCleanupAuthMode, DeviceCleanupFailureKind, DeviceCleanupRemoteOutcome,
};

use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};

use crate::account::actor::{AccountActor, AccountActorHandle, AccountMessage};
use crate::account::profile::AVATAR_DOWNLOAD_CONCURRENCY;
use crate::account::test_support::{inspect_session_runtime, login_gated_actor, test_request_id};
use crate::account::verification::INCOMING_VERIFICATION_FLOW_ID_BASE;
use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
use koushi_protocol::command::AccountCommand;

use crate::executor;

use crate::link_preview::LinkPreviewContext;
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};

use crate::store::StoreActor;
use koushi_store::CredentialStoreBackend;

use crate::timeline::NavigationProjectionIngress;

use tempfile::tempdir;

async fn next_device_cleanup_actions(
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) -> Vec<AppAction> {
    executor::timeout(Duration::from_secs(2), async {
        loop {
            let actions = action_rx
                .recv()
                .await
                .expect("device cleanup action channel");
            if actions.iter().any(|action| {
                matches!(
                    action,
                    AppAction::DeviceCleanupRemoteStarted { .. }
                        | AppAction::DeviceCleanupUiaRequired { .. }
                        | AppAction::DeviceCleanupRemoteSettled { .. }
                        | AppAction::DeviceCleanupRemoteFailed { .. }
                        | AppAction::DeviceCleanupLocalResetFailed { .. }
                        | AppAction::DeviceCleanupCompleted { .. }
                )
            }) {
                return actions;
            }
        }
    })
    .await
    .expect("device cleanup action timeout")
}

#[tokio::test]
async fn device_cleanup_remote_failure_preserves_the_provisional_session() {
    let (handle, mut action_rx) = login_gated_actor().await;
    handle
        .send(AccountMessage::ConfigureDeviceCleanupResults {
            results: vec![Err(DeviceCleanupFailureKind::Network)],
        })
        .await;
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 301,
    };

    handle
        .send(AccountMessage::Command(
            AccountCommand::StartDeviceCleanup { request_id },
        ))
        .await;

    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupRemoteStarted {
            request_id: 301,
            auth_mode: DeviceCleanupAuthMode::Legacy,
        }]
    ));
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupRemoteFailed {
            request_id: 301,
            auth_mode: DeviceCleanupAuthMode::Legacy,
            kind: DeviceCleanupFailureKind::Network,
        }]
    ));
    assert!(
        inspect_session_runtime(&handle).await.0,
        "remote failure must retain the provisional SDK session"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn device_cleanup_uia_and_local_retry_do_not_repeat_remote_cleanup() {
    let (handle, mut action_rx) = login_gated_actor().await;
    handle
        .send(AccountMessage::ConfigureDeviceCleanupResults {
            results: vec![
                Ok(koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired {
                    session: Some("opaque-test-session".to_owned()),
                }),
                Ok(koushi_sdk::MatrixDeviceCleanupOutcome::Settled(
                    DeviceCleanupRemoteOutcome::Success,
                )),
            ],
        })
        .await;
    handle
        .send(AccountMessage::ConfigureCloseStoreResults {
            results: vec![false, true],
        })
        .await;
    let start_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 401,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::StartDeviceCleanup {
                request_id: start_request_id,
            },
        ))
        .await;
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupRemoteStarted {
            request_id: 401,
            ..
        }]
    ));
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupUiaRequired {
            request_id: 401,
            flow_id: 401,
        }]
    ));

    handle
        .send(AccountMessage::Command(
            AccountCommand::SubmitDeviceCleanupUia {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 402,
                },
                flow_id: 401,
                password: koushi_state::AuthSecret::new("test-password"),
            },
        ))
        .await;
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupRemoteSettled {
            request_id: 401,
            outcome: DeviceCleanupRemoteOutcome::Success,
        }]
    ));
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupLocalResetFailed {
            request_id: 401,
            kind: DeviceCleanupFailureKind::LocalData,
        }]
    ));

    let retry_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 403,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::StartDeviceCleanup {
                request_id: retry_request_id,
            },
        ))
        .await;
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupCompleted { request_id: 403 }]
    ));
    assert!(
        !inspect_session_runtime(&handle).await.0,
        "successful local retry must drop the provisional SDK session"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn device_cleanup_local_only_escape_runs_only_after_remote_failure() {
    let (handle, mut action_rx) = login_gated_actor().await;
    handle
        .send(AccountMessage::ConfigureDeviceCleanupResults {
            results: vec![Err(DeviceCleanupFailureKind::Forbidden)],
        })
        .await;
    let start_request_id = RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 501,
    };
    handle
        .send(AccountMessage::Command(
            AccountCommand::StartDeviceCleanup {
                request_id: start_request_id,
            },
        ))
        .await;
    let _ = next_device_cleanup_actions(&mut action_rx).await;
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupRemoteFailed {
            request_id: 501,
            kind: DeviceCleanupFailureKind::Forbidden,
            ..
        }]
    ));

    handle
        .send(AccountMessage::Command(
            AccountCommand::EraseDeviceCleanupLocalDataAnyway {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 502,
                },
            },
        ))
        .await;
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupCompleted { request_id: 502 }]
    ));
    assert!(!inspect_session_runtime(&handle).await.0);
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn provisional_teardown_drops_actor_private_device_cleanup_continuation() {
    let (handle, mut action_rx) = login_gated_actor().await;
    handle
        .send(AccountMessage::ConfigureDeviceCleanupResults {
            results: vec![Ok(koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired {
                session: Some("opaque-test-session".to_owned()),
            })],
        })
        .await;
    handle
        .send(AccountMessage::Command(
            AccountCommand::StartDeviceCleanup {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 601,
                },
            },
        ))
        .await;
    let _ = next_device_cleanup_actions(&mut action_rx).await;
    assert!(matches!(
        next_device_cleanup_actions(&mut action_rx).await.as_slice(),
        [AppAction::DeviceCleanupUiaRequired {
            request_id: 601,
            flow_id: 601,
        }]
    ));
    assert!(inspect_pending_device_cleanup(&handle).await);

    handle
        .send(AccountMessage::RejectProvisionalSession {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 602,
            },
        })
        .await;
    executor::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                action_rx.recv().await.as_deref(),
                Some([AppAction::LogoutFinished])
            ) {
                break;
            }
        }
    })
    .await
    .expect("provisional rejection settles");

    assert!(
        !inspect_pending_device_cleanup(&handle).await,
        "teardown must discard actor-private UIAA continuation state"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

async fn inspect_pending_device_cleanup(handle: &AccountActorHandle) -> bool {
    let (response, result) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::InspectPendingDeviceCleanup { response })
            .await
    );
    result.await.expect("pending device cleanup inspection")
}

#[tokio::test]
async fn reset_local_data_clears_current_account_persistence_and_signs_out_locally() {
    use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let key_id = SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@reset-user:example.test".to_owned(),
        device_id: "RESETDEVICE".to_owned(),
    };
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path())),
        data_dir.path(),
    );
    let store_config = store
        .account_store_config(&key_id)
        .expect("seed local unlock secret");
    let account_root = store_config
        .store_config
        .path()
        .parent()
        .expect("store path should have account root")
        .to_path_buf();
    std::fs::create_dir_all(store_config.store_config.path()).expect("create store dir");
    std::fs::write(
        store_config.store_config.path().join("sentinel"),
        b"local data",
    )
    .expect("write local store sentinel");
    let mut read_state = ReadStateEngine::new(1);
    read_state.admit(
        1,
        ReadStateKey::PublicUnthreaded {
            room_id: "!reset-room:example.test".to_owned(),
        },
        ReadTarget::new("$reset-event".to_owned()),
        ReadWaiterId::new(1),
    );
    store
        .save_read_state_outbox(&key_id, &read_state.persistence_snapshot())
        .expect("seed read-state outbox");
    store
        .credential_backend()
        .save_matrix_session(&key_id, &StoredMatrixSession::new("{\"redacted\":true}"))
        .expect("seed session");
    store
        .credential_backend()
        .remember_saved_session(&key_id)
        .expect("seed saved-session index");
    store
        .credential_backend()
        .save_last_session(&key_id)
        .expect("seed last-session pointer");
    assert_eq!(
        store.probe_local_encryption_health(&key_id),
        koushi_state::LocalEncryptionHealth::Healthy
    );

    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _) = broadcast::channel(16);
    let (self_tx, command_rx) = mpsc::channel(16);
    let data_dir_path = store.data_dir().to_path_buf();
    let account_work = crate::account_work::AccountWorkScheduler::default();
    let room_actor = crate::room::RoomActor::spawn_with_account_work(
        action_tx.clone(),
        event_tx.clone(),
        crate::SlidingSyncDiagnostics::default(),
        account_work.clone(),
    );
    let (navigation_projection, navigation_projection_rx) = NavigationProjectionIngress::channel();
    let (focused_projection_tx, _focused_projection_rx) = mpsc::unbounded_channel();
    let timeline_manager = crate::timeline::TimelineManagerActor::spawn(
        action_tx.clone(),
        event_tx.clone(),
        Some(data_dir_path.clone()),
        account_work.clone(),
        Some(navigation_projection_rx),
        Some(focused_projection_tx.clone()),
    );
    let mut actor = AccountActor {
        session: None,
        session_key_id: Some(key_id.clone()),
        locked_session_record: None,
        provisional_persistable: None,
        sliding_sync_positive_evidence: None,
        sliding_sync_account_epoch: 0,
        sliding_sync_request_id: 0,
        pending_sliding_sync_admission: None,
        pending_sliding_sync_retry: None,
        stored_sliding_sync_admission: None,
        sliding_sync_discovery_task: None,
        sliding_sync_revalidation_pending: None,
        sliding_sync_revalidation_request: None,
        sliding_sync_diagnostics: crate::SlidingSyncDiagnostics::default(),
        native_artifacts: Arc::new(crate::native_artifact::RejectingNativeArtifactPort),
        session_promoted: false,
        trust_generation: 0,
        trust_observer: None,
        trust_recheck_task: None,
        trust_recheck_pending: false,
        current_session_status_task: None,
        current_session_status_request: None,
        secure_backup_ready: false,
        recovery_key_delivery_pending: false,
        secure_backup_inspection_task: None,
        secure_backup_monitor_task: None,
        secure_backup_monitor_serial: 0,
        secure_backup_inspection_pending: false,
        sync_connectivity_proven: false,
        secure_backup_retry_attempt: 0,
        secure_backup_recovery_epoch: false,
        secure_backup_recovery_reset_consumed: false,
        secure_backup_observer: None,
        verification_method_discovery_task: None,
        verification_method_discovery_admission_task: None,
        verification_method_discovery_serial: 0,
        verification_method_discovery_failed: false,
        recovery_task: None,
        pending_recovery_completion: None,
        recovery_trust_settlement_task: None,
        provisional_encryption_sync: None,
        provisional_encryption_sync_ready: false,
        encryption_sync_permit: koushi_sdk::new_encryption_sync_permit_owner(),
        pending_ready_events: Vec::new(),
        pending_trust_transition: None,
        next_trust_transition_id: 0,
        pending_session_teardown: None,
        next_teardown_generation: 0,
        teardown_retry_task: None,
        lifecycle_probe: None,
        residency_install_gap: None,
        #[cfg(any(test, feature = "test-hooks"))]
        residency_teardown_gap: None,
        #[cfg(any(test, feature = "test-hooks"))]
        residency_preserve_room_session: false,
        trust_observation_override: std::sync::Mutex::new(None),
        trust_observation_is_synthetic: false,
        recovery_download_override: std::sync::Mutex::new(None),
        recovery_result_override: std::sync::Mutex::new(None),
        close_store_results: std::collections::VecDeque::new(),
        account_management_discovery_override: std::sync::Mutex::new(None),
        device_cleanup_results: std::collections::VecDeque::new(),
        store: store.clone(),
        action_tx,
        event_tx,
        command_rx,
        self_tx,
        sync_actor: None,
        sync_generation: Arc::new(AtomicU64::new(0)),
        room_actor,
        timeline_manager,
        read_persistence_task: None,
        read_persistence_session_generation: 0,
        navigation_projection,
        focused_projection_tx,
        account_work,
        activity_resolution_task: None,
        data_dir: data_dir_path,
        link_preview_policy: LinkPreviewContext::default(),
        send_read_receipts: true,
        pending_oidc_login: None,
        oidc_completion_override: None,
        search_actor: None,
        threads_list_actor: None,
        recovery_observer: None,
        identity_reset_handle: None,
        identity_reset_flow_id: None,
        identity_reset_timeout_task: None,
        pending_uia_operations: BTreeMap::new(),
        pending_device_cleanup: None,
        verification_request: None,
        sas_verification: None,
        own_user_verification: None,
        sas_waiting_for: None,
        verification_request_observer: None,
        sas_verification_observer: None,
        sas_timeout_task: None,
        synthetic_verification: None,
        incoming_verification_observer: None,
        incoming_verification_session_generation: 0,
        session_change_observer: None,
        account_hydration_task: None,
        account_management_discovery_task: None,
        account_management_discovery_generation: 0,
        account_hydration_generation: 0,
        composer_draft_leases: Arc::new(ComposerDraftLeaseRegistry::new()),
        next_incoming_verification_sequence: INCOMING_VERIFICATION_FLOW_ID_BASE,
        pending_crawler_notification: None,
        avatar_cache: HashMap::new(),
        avatar_inflight: HashMap::new(),
        avatar_download_semaphore: Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY)),
        avatar_fetch_tasks: tokio::task::JoinSet::new(),
        avatar_session_generation: 0,
    };
    let request_id = test_request_id();

    actor.handle_reset_local_data(request_id).await;

    let actions = action_rx.recv().await.expect("reset actions");
    assert!(
        matches!(
            actions.as_slice(),
            [
                AppAction::ResetLocalDataCompleted { request_id: 1 },
                AppAction::LogoutFinished,
            ]
        ),
        "reset must complete and locally sign out, got {actions:?}"
    );
    assert!(!account_root.exists(), "account root should be removed");
    assert!(
        store
            .load_read_state_outbox(&key_id)
            .expect("removed read-state outbox reads as empty")
            .is_empty()
    );

    let check_backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    assert!(koushi_key::is_missing_credential_error(
        &check_backend
            .load_matrix_session(&key_id)
            .expect_err("matrix session should be deleted")
    ));
    assert!(
        check_backend
            .load_saved_sessions()
            .expect("saved-session index")
            .sessions()
            .is_empty()
    );
    assert_eq!(
        check_backend
            .load_last_session()
            .expect("last-session pointer"),
        None
    );
    let check_store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path())),
        data_dir.path(),
    );
    assert_eq!(
        check_store.probe_local_encryption_health(&key_id),
        koushi_state::LocalEncryptionHealth::MissingCredential
    );
}
