use std::{sync::Arc, time::Duration};

use koushi_key::StoredMatrixSession;
use koushi_protocol::SessionKeyId;
use koushi_sdk::PersistableMatrixSession;
use koushi_state::{
    AppAction, LoginAttemptId, LoginRequest, SlidingSyncAdmission, SlidingSyncAdmissionSource,
    SlidingSyncCapabilityResult, SlidingSyncPositiveEvidence,
};

use tokio::sync::{broadcast, mpsc, oneshot};

use super::{
    SESSION_NOT_FOUND_FAILURE, ServerLogoutOutcome, SessionInvalidationReason,
    run_session_change_observation, wait_for_server_logout_best_effort,
};
use crate::account::actor::{AccountActor, AccountActorHandle, AccountMessage};
use crate::account::test_support::{
    acknowledge_next_verified_projection, assert_no_logout_finished, configure_verified_trust,
    consume_initial_unknown_trust_projection, inspect_session_runtime, inspect_sync_owners,
    recv_account_action_with_sliding_sync_effects, recv_probe_with_sliding_sync_effects,
    shutdown_and_ack, spawn_actor_with_dirs, spawn_named_quarantine_password_server,
    spawn_named_quarantine_password_server_with_controls, spawn_quarantine_password_server,
    test_request_id,
};
use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
use crate::executor;
use koushi_protocol::command::AccountCommand;
use koushi_protocol::event::{AccountEvent, CoreEvent};

use crate::link_preview::LinkPreviewContext;
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::{AccountKey, RequestId, RuntimeConnectionId};

use crate::store::{StoreActor, session_key_id_from_info};
use koushi_store::CredentialStoreBackend;

use tempfile::tempdir;

/// Network-free: restoring an account with no stored session must emit the
/// redacted not-found failure AND project `RestoreSessionNotFound` so the
/// reducer returns AppState to SignedOut. Same contract for SwitchAccount.
#[tokio::test]
async fn restore_and_switch_of_unknown_account_emit_not_found() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let store = StoreActor::with_backend(
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path())),
        data_dir.path(),
    );

    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let handle = AccountActor::spawn(
        store,
        action_tx,
        event_tx,
        LinkPreviewContext::default(),
        Arc::new(ComposerDraftLeaseRegistry::new()),
    );

    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 1,
    };
    let account_key = AccountKey("@nobody:example.test".to_owned());

    for command in [
        AccountCommand::RestoreSession {
            request_id,
            account_key: account_key.clone(),
        },
        AccountCommand::SwitchAccount {
            request_id,
            account_key: account_key.clone(),
        },
    ] {
        assert!(handle.send(AccountMessage::Command(command)).await);

        let actions = action_rx.recv().await.expect("reducer actions");
        assert!(
            matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
            "not-found must project RestoreSessionNotFound, got {actions:?}"
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn hard_logout_cleanup_is_bounded_and_deletes_account_persistence() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let baseline_files = recursive_file_count(data_dir.path());
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    configure_verified_trust(&handle).await;
    handle
        .send(AccountMessage::AttachLifecycleProbe { probe_tx })
        .await;
    handle
        .send(AccountMessage::ConfigureCloseStoreResults {
            results: vec![false, true],
        })
        .await;
    let login_request_id = test_request_id();
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: login_request_id,
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}
    let files_before_logout = recursive_file_count(data_dir.path());
    assert!(files_before_logout > baseline_files);

    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 2,
    };
    handle
        .send(AccountMessage::Command(AccountCommand::Logout {
            request_id,
        }))
        .await;
    recv_probe_with_sliding_sync_effects(
        &handle,
        &mut action_rx,
        &mut probe_rx,
        "session_store_close_retrying",
    )
    .await;
    assert_eq!(recursive_file_count(data_dir.path()), files_before_logout);
    assert_no_logout_finished(&mut action_rx);

    handle
        .send(AccountMessage::RetrySessionTeardown { generation: 1 })
        .await;
    assert_eq!(probe_rx.recv().await, Some("session_store_closed"));
    assert_eq!(probe_rx.recv().await, Some("session_persistence_deleted"));
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::LogoutFinished])
    ) {}
    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    assert!(
        backend
            .load_last_session()
            .expect("last pointer after logout")
            .is_none()
    );
    assert!(
        backend
            .load_saved_sessions()
            .expect("saved sessions after logout")
            .sessions()
            .is_empty()
    );
    assert_eq!(recursive_file_count(data_dir.path()), baseline_files);
    loop {
        if let CoreEvent::Account(AccountEvent::LoggedOut {
            request_id: terminal,
            ..
        }) = event_rx.recv().await.expect("logout event")
        {
            assert_eq!(terminal, request_id);
            break;
        }
    }
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn password_login_names_an_unnamed_device_with_the_platform_default() {
    let (homeserver, rename_bodies) = spawn_device_naming_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: test_request_id(),
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}

    // The cosmetic device rename ran exactly once with the platform
    // default. Exact JSON equality proves the body is only the display
    // name — no username, device id, token, or other private identifier.
    let bodies = rename_bodies.lock().expect("rename record");
    assert_eq!(bodies.len(), 1, "device rename should run once");
    let parsed: serde_json::Value =
        serde_json::from_str(&bodies[0]).expect("rename body should be JSON");
    assert_eq!(
        parsed,
        serde_json::json!({ "display_name": "Koushi on Linux" })
    );
    drop(bodies);
    shutdown_and_ack(&handle).await;
    while let Ok(event) = event_rx.try_recv() {
        assert!(!matches!(
            event,
            CoreEvent::Account(AccountEvent::LoggedOut { .. })
        ));
    }
}

#[tokio::test]
async fn password_login_preserves_a_customized_device_name() {
    let (homeserver, rename_bodies) = spawn_device_naming_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: test_request_id(),
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("My Laptop".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Macos,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}

    let bodies = rename_bodies.lock().expect("rename record");
    assert_eq!(
        bodies.len(),
        0,
        "a customized device name must not be rewritten"
    );
    drop(bodies);
    shutdown_and_ack(&handle).await;
    while let Ok(event) = event_rx.try_recv() {
        assert!(!matches!(
            event,
            CoreEvent::Account(AccountEvent::LoggedOut { .. })
        ));
    }
}

#[tokio::test]
async fn password_quarantine_persists_no_credentials_and_restart_is_signed_out() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: Some("Quarantine Test".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
    );
    let first_actions = action_rx.recv().await;
    assert!(
        matches!(
            first_actions.as_deref(),
            Some([AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::NewLogin { .. },
                ..
            }])
        ),
        "unexpected first login actions: {first_actions:?}"
    );
    assert!(matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::SlidingSyncCapabilityCheckCompleted {
            result: SlidingSyncCapabilityResult::Supported { .. },
            ..
        }]
    ));
    let actions = action_rx.recv().await.expect("provisional login action");
    assert!(matches!(
        actions.as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ));

    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    assert!(
        backend
            .load_last_session()
            .expect("last pointer read")
            .is_none()
    );
    assert!(
        backend
            .load_saved_sessions()
            .expect("saved index read")
            .sessions()
            .is_empty()
    );

    let _ = handle.send(AccountMessage::Shutdown).await;
    let (restarted, mut restarted_actions, _events) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    assert!(
        restarted
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession { request_id }
            ))
            .await
    );
    assert!(matches!(
        restarted_actions.recv().await.as_deref(),
        Some([AppAction::RestoreSessionNotFound])
    ));
    let _ = restarted.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn oidc_completion_installs_only_a_provisional_quarantined_session() {
    let homeserver = spawn_quarantine_password_server();
    let login_session = koushi_sdk::login_with_password_with_store(
        &LoginRequest {
            homeserver: homeserver.clone(),
            username: "fixture-user".to_owned(),
            password: koushi_state::AuthSecret::new("synthetic-password"),
            device_display_name: Some("OIDC Quarantine Test".to_owned()),
        },
        None,
    )
    .await
    .expect("fixture login");

    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (_trust_tx, trust_rx) = mpsc::unbounded_channel();
    let updates = futures_util::stream::unfold(trust_rx, |mut rx| async move {
        rx.recv().await.map(|trust| (trust, rx))
    });
    assert!(
        handle
            .send(AccountMessage::ConfigureTrustObservation {
                observation: koushi_sdk::CurrentDeviceTrustObservation {
                    current: koushi_state::CurrentDeviceTrustState::Unknown,
                    updates: Box::pin(updates),
                },
            })
            .await
    );
    let start_request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::ConfigureOidcCompletion {
                start_request_id,
                homeserver: homeserver.clone(),
                session: login_session,
            })
            .await
    );
    let completion_request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(41),
        sequence: 7,
    };
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::CompleteOidcLogin {
                request_id: completion_request_id,
                callback_url: "http://127.0.0.1/callback?code=fixture&state=fixture".to_owned(),
                platform: koushi_state::DisplayPlatform::Linux,
            },))
            .await
    );
    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::AuthenticationStarted {
            attempt_id,
            homeserver: projected_homeserver,
        }]) if *attempt_id == LoginAttemptId::new(41, 7)
            && projected_homeserver == &homeserver
    ));
    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::SlidingSyncCapabilityCheckStarted {
            admission: SlidingSyncAdmission::NewLogin { attempt_id },
            ..
        }]) if *attempt_id == LoginAttemptId::new(41, 7)
    ));
    assert!(matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::SlidingSyncCapabilityCheckCompleted {
            result: SlidingSyncCapabilityResult::Supported { .. },
            ..
        }]
    ));
    assert!(matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { attempt_id, .. }]
            if *attempt_id == LoginAttemptId::new(41, 7)
    ));
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, false, false, true)
    );

    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    assert!(backend.load_last_session().expect("pointer read").is_none());
    assert!(
        backend
            .load_saved_sessions()
            .expect("index read")
            .sessions()
            .is_empty()
    );
    assert!(
        executor::timeout(Duration::from_millis(100), async {
            loop {
                match event_rx.recv().await.expect("event stream") {
                    CoreEvent::Account(AccountEvent::LoggedIn { .. }) | CoreEvent::Sync(_) => {
                        return;
                    }
                    _ => {}
                }
            }
        })
        .await
        .is_err(),
        "OIDC completion escaped quarantine before Verified"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn verified_warm_restore_skips_restricted_and_full_state_preparation() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    configure_verified_trust(&handle).await;
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: test_request_id(),
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}

    assert_eq!(
        inspect_sync_owners(&handle).await,
        (false, false, false),
        "authoritative Verified restore must not start restricted or promotion sync"
    );

    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    assert_eq!(
        inspect_sync_owners(&handle).await,
        (false, false, true),
        "normal sync must be the sole owner after Ready projection acknowledgement"
    );
    let snapshot = koushi_diagnostics::test_support::detail_snapshot();
    let stages = snapshot.records[diagnostic_start..]
        .iter()
        .filter(|record| record.event.source == "core.verification_admission")
        .map(|record| record.event.stage)
        .collect::<Vec<_>>();
    let mut remaining = stages.as_slice();
    for expected in [
        "provisional_encryption_sync_skipped",
        "ready_projection_dispatched",
        "normal_sync_started",
    ] {
        let index = remaining
            .iter()
            .position(|stage| *stage == expected)
            .unwrap_or_else(|| panic!("missing ordered admission stage {expected}: {stages:?}"));
        remaining = &remaining[index + 1..];
    }
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn verified_offline_warm_restore_reaches_ready_without_network_catch_up() {
    let (homeserver, offline, sliding_sync_supported) =
        spawn_controllable_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    let key_id = SessionKeyId {
        homeserver: homeserver.clone(),
        user_id: "@fixture-user:example.invalid".to_owned(),
        device_id: "FIXTUREDEVICE".to_owned(),
    };
    let store = StoreActor::with_backend(backend.clone(), data_dir.path());
    let store_config = store
        .account_store_config(&key_id)
        .expect("fixture persistent store");
    let login = koushi_sdk::login_with_password_with_new_device(
        &LoginRequest {
            homeserver,
            username: "fixture-user".to_owned(),
            password: koushi_state::AuthSecret::new("synthetic-password"),
            device_display_name: Some("Offline Restore Test".to_owned()),
        },
        &store_config.store_config,
        &key_id.device_id,
    )
    .await
    .expect("fixture login");
    assert_eq!(session_key_id_from_info(&login.info), key_id);
    let stored = StoredMatrixSession::new(
        login
            .persistable_session()
            .expect("persistable")
            .with_sliding_sync_positive_evidence(SlidingSyncPositiveEvidence { observed_at_ms: 11 })
            .to_json()
            .expect("json"),
    );
    drop(login);
    backend
        .save_matrix_session(&key_id, &stored)
        .expect("session seed");
    backend.remember_saved_session(&key_id).expect("index seed");
    backend.save_last_session(&key_id).expect("pointer seed");

    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    configure_verified_trust(&handle).await;
    offline.store(true, std::sync::atomic::Ordering::SeqCst);
    handle
        .send(AccountMessage::Command(
            AccountCommand::RestoreLastSession {
                request_id: test_request_id(),
            },
        ))
        .await;
    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::SlidingSyncCapabilityCheckStarted {
            admission: SlidingSyncAdmission::StoredSessionRestore { .. },
            positive_evidence: Some(_),
            ..
        }])
    ));
    let (offline_epoch, offline_request_id) = match action_rx.recv().await.as_deref() {
        Some(
            [
                AppAction::SlidingSyncCapabilityCheckCompleted {
                    account_epoch,
                    request_id,
                    result: SlidingSyncCapabilityResult::Unreachable,
                },
            ],
        ) => (*account_epoch, *request_id),
        other => panic!("expected unreachable capability result, got {other:?}"),
    };
    handle
        .send(AccountMessage::ContinueSlidingSyncAdmission {
            account_epoch: offline_epoch,
            request_id: offline_request_id,
            source: SlidingSyncAdmissionSource::PositiveCache,
        })
        .await;
    handle
        .send(AccountMessage::ScheduleSlidingSyncCapabilityRevalidation {
            account_epoch: offline_epoch,
        })
        .await;
    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::RestoreSessionSucceeded(_)])
    ));

    offline.store(false, std::sync::atomic::Ordering::SeqCst);
    sliding_sync_supported.store(false, std::sync::atomic::Ordering::SeqCst);
    executor::timeout(
        Duration::from_secs(1),
        acknowledge_next_verified_projection(&handle, &mut action_rx),
    )
    .await
    .expect("offline verified restore must reach Ready without network catch-up");
    let (account_epoch, blocked_request_id) = match action_rx.recv().await.as_deref() {
        Some(
            [
                AppAction::SlidingSyncCapabilityRevalidationStarted {
                    account_epoch,
                    request_id,
                },
            ],
        ) => (*account_epoch, *request_id),
        other => panic!("expected revalidation start, got {other:?}"),
    };
    let revalidation_result = loop {
        let actions = action_rx.recv().await.expect("revalidation action");
        if let [AppAction::SlidingSyncCapabilityRevalidationCompleted { result, .. }] =
            actions.as_slice()
        {
            break result.clone();
        }
    };
    assert_eq!(
        revalidation_result,
        SlidingSyncCapabilityResult::Unsupported
    );
    assert_eq!(
        inspect_sync_owners(&handle).await,
        (false, false, true),
        "actor must await the reducer-accepted settlement effect"
    );
    handle
        .send(AccountMessage::SettleSlidingSyncCapabilityRevalidation {
            account_epoch,
            request_id: blocked_request_id,
            result: SlidingSyncCapabilityResult::Unsupported,
        })
        .await;
    assert_eq!(inspect_sync_owners(&handle).await, (false, false, false));
    handle
        .send(AccountMessage::Command(
            AccountCommand::RetrySlidingSyncCapability {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 2,
                },
            },
        ))
        .await;
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if matches!(
            actions.as_slice(),
            [AppAction::SlidingSyncCapabilityRetryAccepted {
                account_epoch: accepted_epoch,
                blocked_request_id: accepted_request_id,
                ..
            }] if *accepted_epoch == account_epoch && *accepted_request_id == blocked_request_id
        ) {
            break;
        }
    }
    loop {
        let actions = action_rx.recv().await.expect("retry start action");
        if matches!(
            actions.as_slice(),
            [AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::StoredSessionRestore { .. },
                ..
            }]
        ) {
            break;
        }
    }
    assert_eq!(inspect_sync_owners(&handle).await, (false, false, false));
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn provisional_rejection_deletes_keyed_store_before_signed_out_ack() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    assert!(
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await
    );
    let baseline_files = recursive_file_count(data_dir.path());
    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: Some("Quarantine Test".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
    );
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if matches!(actions.as_slice(), [AppAction::LoginSucceeded { .. }]) {
            break;
        }
    }
    assert!(
        recursive_file_count(data_dir.path()) > baseline_files,
        "keyed store was not created"
    );

    assert!(
        handle
            .send(AccountMessage::RejectProvisionalSession { request_id })
            .await
    );
    loop {
        let actions = action_rx.recv().await.expect("rejection action");
        if matches!(actions.as_slice(), [AppAction::LogoutFinished]) {
            assert_eq!(
                probe_rx.try_recv(),
                Ok("trust_observer_terminated"),
                "LogoutFinished preceded trust-observer termination"
            );
            assert_eq!(
                probe_rx.try_recv(),
                Ok("provisional_encryption_sync_terminated"),
                "LogoutFinished preceded restricted-sync termination"
            );
            assert_eq!(
                recursive_file_count(data_dir.path()),
                baseline_files,
                "SignedOut ack preceded keyed-store deletion"
            );
            break;
        }
    }
    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    assert!(backend.load_last_session().expect("pointer read").is_none());
    assert!(
        backend
            .load_saved_sessions()
            .expect("index read")
            .sessions()
            .is_empty()
    );
    shutdown_and_ack(&handle).await;
    let (restarted, mut restarted_actions, _restarted_events) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let restore_id = RequestId {
        connection_id: RuntimeConnectionId(19),
        sequence: 1,
    };
    assert!(
        restarted
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession {
                    request_id: restore_id,
                },
            ))
            .await
    );
    assert!(matches!(
        restarted_actions.recv().await.as_deref(),
        Some([AppAction::RestoreSessionNotFound])
    ));
    shutdown_and_ack(&restarted).await;
}

#[tokio::test]
async fn teardown_close_failure_retries_without_early_ack_and_preserves_request_correlation() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    handle
        .send(AccountMessage::AttachLifecycleProbe { probe_tx })
        .await;
    handle
        .send(AccountMessage::ConfigureCloseStoreResults {
            results: vec![false, true],
        })
        .await;
    let original = test_request_id();
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: original,
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("Teardown Retry Test".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}
    handle
        .send(AccountMessage::RejectProvisionalSession {
            request_id: original,
        })
        .await;
    while probe_rx.recv().await != Some("session_store_close_retrying") {}
    assert_no_logout_finished(&mut action_rx);

    let later = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(77),
        sequence: 2,
    };
    handle
        .send(AccountMessage::RejectProvisionalSession { request_id: later })
        .await;
    loop {
        if let CoreEvent::OperationFailed {
            request_id,
            failure,
        } = event_rx.recv().await.expect("failure event")
            && request_id == later
        {
            assert_eq!(failure, CoreFailure::SessionRequired);
            break;
        }
    }
    handle
        .send(AccountMessage::RetrySessionTeardown { generation: 999 })
        .await;
    assert_no_logout_finished(&mut action_rx);
    handle
        .send(AccountMessage::RetrySessionTeardown { generation: 1 })
        .await;
    assert_eq!(probe_rx.recv().await, Some("session_store_closed"));
    assert_eq!(probe_rx.recv().await, Some("session_persistence_deleted"));
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::LogoutFinished])
    ) {}
    loop {
        if let CoreEvent::Account(AccountEvent::LoggedOut { request_id, .. }) =
            event_rx.recv().await.expect("logout event")
        {
            assert_eq!(request_id, original);
            break;
        }
    }
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn permanent_close_failures_never_ack_before_a_success_barrier() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    handle
        .send(AccountMessage::AttachLifecycleProbe { probe_tx })
        .await;
    handle
        .send(AccountMessage::ConfigureCloseStoreResults {
            results: vec![false; 16],
        })
        .await;
    let request_id = test_request_id();
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}
    handle
        .send(AccountMessage::RejectProvisionalSession { request_id })
        .await;
    for _ in 0..4 {
        while probe_rx.recv().await != Some("session_store_close_retrying") {}
        assert_no_logout_finished(&mut action_rx);
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
    }
    assert_no_logout_finished(&mut action_rx);
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn replacement_install_waits_for_provisional_tasks_to_terminate() {
    let first_homeserver = spawn_quarantine_password_server();
    let second_homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    assert!(
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await
    );
    for homeserver in [first_homeserver, second_homeserver] {
        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture-user".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: Some("Replacement Barrier Test".to_owned()),
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
        );
        loop {
            if matches!(
                recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                    .await
                    .as_slice(),
                [AppAction::LoginSucceeded { .. }]
            ) {
                break;
            }
        }
    }
    assert_eq!(probe_rx.try_recv(), Ok("trust_observer_terminated"));
    assert_eq!(
        probe_rx.try_recv(),
        Ok("provisional_encryption_sync_terminated")
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn replacement_close_failure_holds_incoming_until_generation_retry_succeeds() {
    let first_homeserver = spawn_quarantine_password_server();
    let second_homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    handle
        .send(AccountMessage::AttachLifecycleProbe { probe_tx })
        .await;
    let first_request = test_request_id();
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: first_request,
            request: LoginRequest {
                homeserver: first_homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    while !matches!(
        recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::LoginSucceeded { .. }]
    ) {}
    handle
        .send(AccountMessage::ConfigureCloseStoreResults {
            results: vec![false, true],
        })
        .await;
    let replacement_request = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(2),
        sequence: 2,
    };
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: replacement_request,
            request: LoginRequest {
                homeserver: second_homeserver.clone(),
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    recv_probe_with_sliding_sync_effects(
        &handle,
        &mut action_rx,
        &mut probe_rx,
        "session_store_close_retrying",
    )
    .await;
    assert_no_login_succeeded_for(&mut action_rx, &second_homeserver);
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (false, false, false, false)
    );

    let later = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(3),
        sequence: 3,
    };
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: later,
            request: LoginRequest {
                homeserver: "http://127.0.0.1:9".to_owned(),
                username: "later".to_owned(),
                password: koushi_state::AuthSecret::new("not-used"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    loop {
        if let CoreEvent::OperationFailed {
            request_id,
            failure,
        } = event_rx.recv().await.expect("later rejection")
            && request_id == later
        {
            assert_eq!(failure, CoreFailure::SessionRequired);
            break;
        }
    }
    handle
        .send(AccountMessage::RetrySessionTeardown { generation: 999 })
        .await;
    assert_no_login_succeeded_for(&mut action_rx, &second_homeserver);
    handle
        .send(AccountMessage::RetrySessionTeardown { generation: 1 })
        .await;
    while probe_rx.recv().await != Some("replacement_teardown_complete") {}
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
        if matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { info, .. }] if info.homeserver == second_homeserver
        ) {
            break;
        }
    }
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, false, false, true)
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn real_store_switch_a_to_b_preserves_both_accounts_and_switches_back() {
    let server_a = spawn_named_quarantine_password_server("@alpha:example.invalid", "DEVICEA");
    let server_b = spawn_named_quarantine_password_server("@beta:example.invalid", "DEVICEB");
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    for (sequence, homeserver) in [(1, server_a.clone()), (2, server_b.clone())] {
        configure_verified_trust(&handle).await;
        let request_id = RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(9),
            sequence,
        };
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver,
                    username: "fixture".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    }

    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    let saved = backend.load_saved_sessions().expect("saved index");
    assert_eq!(saved.sessions().len(), 2);
    let alpha_key = saved
        .sessions()
        .iter()
        .find(|key| key.user_id == "@alpha:example.invalid")
        .expect("alpha saved")
        .clone();
    let beta_key = saved
        .sessions()
        .iter()
        .find(|key| key.user_id == "@beta:example.invalid")
        .expect("beta saved")
        .clone();
    assert!(backend.load_matrix_session(&alpha_key).is_ok());
    assert!(backend.load_matrix_session(&beta_key).is_ok());

    for (sequence, user_id) in [(3, "@alpha:example.invalid"), (4, "@beta:example.invalid")] {
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::SwitchAccount {
                request_id: RequestId {
                    connection_id: koushi_protocol::ids::RuntimeConnectionId(9),
                    sequence,
                },
                account_key: AccountKey(user_id.to_owned()),
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        let saved = backend
            .load_saved_sessions()
            .expect("saved index after switch");
        assert_eq!(saved.sessions().len(), 2);
        assert!(backend.load_matrix_session(&alpha_key).is_ok());
        assert!(backend.load_matrix_session(&beta_key).is_ok());
        assert_eq!(
            backend
                .load_last_session()
                .expect("last pointer after switch")
                .expect("last pointer present")
                .user_id,
            user_id
        );
    }
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn same_key_replacement_preserves_open_store_and_restores_again_once() {
    let homeserver =
        spawn_named_quarantine_password_server("@same-key:example.invalid", "SAMEDEVICE");
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    for sequence in [1, 2] {
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: RequestId {
                    connection_id: koushi_protocol::ids::RuntimeConnectionId(11),
                    sequence,
                },
                request: LoginRequest {
                    homeserver: homeserver.clone(),
                    username: "same-key".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    }
    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    let saved = backend.load_saved_sessions().expect("saved same-key index");
    assert_eq!(saved.sessions().len(), 1);
    let key_id = saved.sessions()[0].clone();
    assert!(backend.load_matrix_session(&key_id).is_ok());
    assert!(recursive_file_count(data_dir.path()) > 0);

    configure_verified_trust(&handle).await;
    handle
        .send(AccountMessage::Command(AccountCommand::SwitchAccount {
            request_id: RequestId {
                connection_id: koushi_protocol::ids::RuntimeConnectionId(11),
                sequence: 3,
            },
            account_key: AccountKey("@same-key:example.invalid".to_owned()),
        }))
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    assert!(backend.load_matrix_session(&key_id).is_ok());
    assert!(recursive_file_count(data_dir.path()) > 0);
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, true, true, true)
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

fn assert_no_login_succeeded_for(action_rx: &mut mpsc::Receiver<Vec<AppAction>>, homeserver: &str) {
    while let Ok(actions) = action_rx.try_recv() {
        assert!(!matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { info, .. }] if info.homeserver == homeserver
        ));
    }
}

async fn recv_until_session_install(
    handle: &AccountActorHandle,
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) -> Vec<AppAction> {
    loop {
        let actions = recv_account_action_with_sliding_sync_effects(handle, action_rx).await;
        if actions.iter().any(|action| {
            matches!(
                action,
                AppAction::LoginSucceeded { .. } | AppAction::RestoreSessionSucceeded(_)
            )
        }) {
            return actions;
        }
        assert!(
            actions.iter().all(|action| matches!(
                action,
                AppAction::SlidingSyncCapabilityCheckStarted { .. }
                    | AppAction::SlidingSyncCapabilityCheckCompleted { .. }
            )),
            "unexpected restore actions: restore_failed={} login_failed={} persistence_failed={}",
            actions
                .iter()
                .any(|action| matches!(action, AppAction::RestoreSessionFailed { .. })),
            actions
                .iter()
                .any(|action| matches!(action, AppAction::LoginFailed { .. })),
            actions
                .iter()
                .any(|action| matches!(action, AppAction::SessionPersistenceFailed { .. }))
        );
    }
}

#[tokio::test]
async fn restore_installs_provisional_without_normal_sync_or_public_ready_event() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    let key_id = SessionKeyId {
        homeserver: homeserver.clone(),
        user_id: "@fixture-user:example.invalid".to_owned(),
        device_id: "FIXTUREDEVICE".to_owned(),
    };
    let store = StoreActor::with_backend(backend.clone(), data_dir.path());
    let store_config = store
        .account_store_config(&key_id)
        .expect("fixture persistent store");
    let login = koushi_sdk::login_with_password_with_new_device(
        &LoginRequest {
            homeserver,
            username: "fixture-user".to_owned(),
            password: koushi_state::AuthSecret::new("synthetic-password"),
            device_display_name: Some("Quarantine Test".to_owned()),
        },
        &store_config.store_config,
        &key_id.device_id,
    )
    .await
    .expect("fixture login");
    assert_eq!(session_key_id_from_info(&login.info), key_id);
    let stored = StoredMatrixSession::new(
        login
            .persistable_session()
            .expect("persistable")
            .to_json()
            .expect("json"),
    );
    drop(login);
    backend
        .save_matrix_session(&key_id, &stored)
        .expect("session seed");
    backend.remember_saved_session(&key_id).expect("index seed");
    backend.save_last_session(&key_id).expect("pointer seed");

    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession { request_id }
            ))
            .await
    );
    assert!(matches!(
        recv_until_session_install(&handle, &mut action_rx)
            .await
            .as_slice(),
        [AppAction::RestoreSessionSucceeded(_)]
    ));
    let persisted = backend
        .load_matrix_session(&key_id)
        .expect("restored credential should remain readable");
    assert!(
        PersistableMatrixSession::from_json(persisted.as_str())
            .expect("persisted restored session")
            .sliding_sync_positive_evidence()
            .is_some(),
        "network support evidence must be durable before trust promotion"
    );
    let public_ready = executor::timeout(Duration::from_millis(100), async {
        loop {
            match event_rx.recv().await.expect("event stream") {
                CoreEvent::Account(AccountEvent::SessionRestored { .. }) | CoreEvent::Sync(_) => {
                    return true;
                }
                _ => {}
            }
        }
    })
    .await;
    assert!(
        public_ready.is_err(),
        "restore escaped quarantine before Verified"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

fn recursive_file_count(path: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                recursive_file_count(&path)
            } else {
                1
            }
        })
        .sum()
}

/// Password-login fixture server that also serves the devices list (with
/// the current device unnamed) and records `PUT /devices/…` rename bodies,
/// so the #474 password-login device-naming path is provable end-to-end.
fn spawn_device_naming_password_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>)
{
    use std::io::{Read, Write};
    let rename_bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorder = std::sync::Arc::clone(&rename_bodies);
    let requested_name = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let name_writer = std::sync::Arc::clone(&requested_name);
    let requested_device = std::sync::Arc::new(std::sync::Mutex::new("FIXTUREDEVICE".to_owned()));
    let device_writer = std::sync::Arc::clone(&requested_device);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("address");
    std::thread::spawn(move || {
        'accept: while let Ok((mut stream, _)) = listener.accept() {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = match stream.read(&mut buffer) {
                    Ok(0) => continue 'accept,
                    Ok(count) => count,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::UnexpectedEof
                        ) =>
                    {
                        continue 'accept;
                    }
                    Err(error) => panic!("read: {error}"),
                };
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request);
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                // Header names are case-insensitive; parse the declared
                // Content-Length so a split segment is never mistaken for
                // the full request.
                let length = text
                    .split("\r\n")
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&request);
            let body = if text.starts_with("GET /_matrix/client/versions ") {
                r#"{"versions":["v1.7"],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#
                    .to_owned()
            } else if text.contains("/_matrix/client/") && text.contains("login") {
                // Remember an explicit initial device name so the devices
                // list can report it back (a customized name must read as
                // present and never be rewritten).
                let requested_name = text
                    .split("\r\n\r\n")
                    .nth(1)
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                    .and_then(|value| {
                        value
                            .get("initial_device_display_name")
                            .and_then(|name| name.as_str())
                            .map(|name| name.to_owned())
                    })
                    .filter(|name| !name.trim().is_empty());
                if let Some(name) = requested_name {
                    *name_writer.lock().unwrap() = Some(name);
                }
                let device_id = text
                    .split("\r\n\r\n")
                    .nth(1)
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                    .and_then(|value| value["device_id"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "FIXTUREDEVICE".to_owned());
                *device_writer.lock().unwrap() = device_id.clone();
                format!(
                    r#"{{"access_token":"fixture-token","device_id":"{device_id}","user_id":"@fixture-user:example.invalid"}}"#
                )
            } else if text.contains("GET /_matrix/client/v3/devices ") {
                // The current device is authoritative; it is unnamed unless
                // the login request explicitly named it.
                let device_id = device_writer.lock().unwrap().clone();
                match name_writer.lock().unwrap().clone() {
                    Some(name) => format!(
                        r#"{{"devices":[{{"device_id":"{device_id}","display_name":"{name}"}}]}}"#
                    ),
                    None => format!(
                        r#"{{"devices":[{{"device_id":"{device_id}","display_name":null}}]}}"#
                    ),
                }
            } else if text.contains("PUT /_matrix/client/v3/devices/") {
                let json_start = text.find("\r\n\r\n").map(|index| index + 4).unwrap_or(0);
                recorder
                    .lock()
                    .unwrap()
                    .push(text[json_start..].trim_end().to_owned());
                r#"{}"#.to_owned()
            } else if text.contains("/_matrix/client/") && text.contains("/keys/query") {
                r#"{"device_keys":{},"failures":{}}"#.to_owned()
            } else if text.contains("/_matrix/client/") && text.contains("/sync") {
                r#"{"next_batch":"batch","device_lists":{"changed":[],"left":[]},"rooms":{"invite":{},"join":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_one_time_keys_count":{}}"#
                    .to_owned()
            } else {
                r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#.to_owned()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write");
        }
    });
    (format!("http://{addr}"), rename_bodies)
}

fn spawn_controllable_quarantine_password_server() -> (
    String,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let offline = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sliding_sync_supported = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let homeserver = spawn_named_quarantine_password_server_with_controls(
        "@fixture-user:example.invalid",
        "FIXTUREDEVICE",
        Some(std::sync::Arc::clone(&offline)),
        None,
        std::sync::Arc::clone(&sliding_sync_supported),
    );
    (homeserver, offline, sliding_sync_supported)
}

#[tokio::test]
async fn quarantine_password_server_outlives_the_legacy_request_budget() {
    let homeserver = spawn_quarantine_password_server();
    let address = homeserver
        .strip_prefix("http://")
        .expect("fixture homeserver scheme")
        .parse::<std::net::SocketAddr>()
        .expect("fixture homeserver address");

    for request_number in 0..300 {
        use std::io::{Read, Write};

        let mut stream = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1))
            .unwrap_or_else(|error| panic!("fixture stopped at request {request_number}: {error}"));
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("fixture read timeout");
        stream
            .write_all(
                    b"GET /_matrix/client/versions HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .expect("fixture request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .unwrap_or_else(|error| panic!("fixture response {request_number} failed: {error}"));
        assert!(
            response.contains(r#""org.matrix.simplified_msc3575":true"#),
            "fixture response {request_number}: {response}"
        );
    }
}

#[tokio::test]
async fn session_change_observer_records_exact_unknown_token_diagnostics_for_both_soft_logout_values()
 {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();

    for soft_logout in [true, false] {
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let (tx, mut receiver) = mpsc::channel(1);
        let (change_tx, change_rx) = broadcast::channel(1);
        let (_stop_tx, stop_rx) = oneshot::channel();
        let task = executor::spawn(run_session_change_observation(change_rx, tx, stop_rx, None));
        let mut unknown_token = matrix_sdk::ruma::api::error::UnknownTokenErrorData::new();
        unknown_token.soft_logout = soft_logout;
        change_tx
            .send(matrix_sdk::SessionChange::UnknownToken(unknown_token))
            .expect("publish synthetic session invalidation");

        match receiver.recv().await.expect("observer message") {
            AccountMessage::SessionInvalidated {
                reason:
                    SessionInvalidationReason::UnknownToken {
                        soft_logout: observed,
                    },
            } => assert_eq!(observed, soft_logout),
            _ => panic!("expected UnknownToken invalidation"),
        }
        task.await.expect("session-change observer task");

        let expected = format!(
            "stage=session_change_received source=matrix_sdk reason=unknown_token soft_logout={soft_logout}"
        );
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
            "missing exact observer diagnostic: {expected}"
        );
    }
}

#[tokio::test]
async fn session_change_observer_forwards_token_rotation_and_keeps_observing() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();

    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let (tx, mut receiver) = mpsc::channel(2);
    let (change_tx, change_rx) = broadcast::channel(2);
    let (_stop_tx, stop_rx) = oneshot::channel();
    let task = executor::spawn(run_session_change_observation(change_rx, tx, stop_rx, None));

    change_tx
        .send(matrix_sdk::SessionChange::TokensRefreshed)
        .expect("publish synthetic token rotation");
    match receiver.recv().await.expect("observer message") {
        AccountMessage::SessionTokensRefreshed => {}
        _ => panic!("expected a token-rotation persistence request"),
    }

    // A rotation must not end the observation: the same session keeps running
    // and its later invalidation still has to reach the actor.
    let mut unknown_token = matrix_sdk::ruma::api::error::UnknownTokenErrorData::new();
    unknown_token.soft_logout = false;
    change_tx
        .send(matrix_sdk::SessionChange::UnknownToken(unknown_token))
        .expect("publish synthetic session invalidation");
    match receiver.recv().await.expect("observer message") {
        AccountMessage::SessionInvalidated {
            reason: SessionInvalidationReason::UnknownToken { soft_logout },
        } => assert!(!soft_logout),
        _ => panic!("expected UnknownToken invalidation after a rotation"),
    }
    task.await.expect("session-change observer task");

    let expected =
        "stage=session_change_received source=matrix_sdk reason=tokens_refreshed".to_owned();
    assert!(
        koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
        "missing exact observer diagnostic: {expected}"
    );
}

#[tokio::test]
async fn admitted_unknown_token_records_exact_lock_diagnostics_for_both_soft_logout_values() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();

    for soft_logout in [true, false] {
        let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
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

        assert!(
            handle
                .send(AccountMessage::SessionInvalidated {
                    reason: SessionInvalidationReason::UnknownToken { soft_logout },
                })
                .await
        );
        loop {
            let actions = action_rx.recv().await.expect("account action");
            if let [
                AppAction::SessionAuthenticationInvalidated {
                    soft_logout: observed,
                },
            ] = actions.as_slice()
            {
                assert_eq!(*observed, soft_logout);
                break;
            }
        }

        let expected = format!(
            "stage=session_invalidated reason=unknown_token soft_logout={soft_logout} action=lock"
        );
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
            "missing exact admission diagnostic: {expected}"
        );
        shutdown_and_ack(&handle).await;
    }
}

#[tokio::test]
async fn unknown_token_before_session_promotion_is_inert_and_not_diagnosed() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;

    let before = inspect_session_runtime(&handle).await;
    assert!(before.0, "the provisional actor must still own a session");
    assert!(!before.1, "the session must not be promoted yet");
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();

    assert!(
        handle
            .send(AccountMessage::SessionInvalidated {
                reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
            })
            .await
    );
    let after = inspect_session_runtime(&handle).await;
    assert!(
        after.0,
        "an unpromoted UnknownToken must retain the session"
    );
    assert!(
        !after.1,
        "an unpromoted UnknownToken must remain unpromoted"
    );
    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !matches!(
                actions.as_slice(),
                [AppAction::SessionAuthenticationInvalidated { .. }]
            ),
            "an unpromoted UnknownToken must not dispatch an authentication lock"
        );
    }
    assert!(
        !koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .any(|record| {
                record.event.source == "core.account" && record.event.stage == "session_invalidated"
            }),
        "an unpromoted UnknownToken must not emit an admitted lock diagnostic"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn unknown_token_fences_an_in_flight_verified_trust_completion() {
    let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;

    assert!(
        handle
            .send(AccountMessage::SessionInvalidated {
                reason: SessionInvalidationReason::UnknownToken { soft_logout: false },
            })
            .await
    );
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::SessionAuthenticationInvalidated { .. }])
    ) {}
    handle
        .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
            generation: 2,
            result: Ok(koushi_state::CurrentDeviceTrustState::Verified),
        })
        .await;
    let _ = inspect_session_runtime(&handle).await;
    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !matches!(
                actions.as_slice(),
                [AppAction::AuthoritativeDeviceTrustChanged {
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                    ..
                }]
            ),
            "stale trust completion must not unlock an invalid authentication session"
        );
    }
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn post_teardown_unknown_token_message_is_inert_and_not_diagnosed() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;

    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::Logout {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 2,
                },
            }))
            .await
    );
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::LogoutFinished])
    ) {}
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();

    assert!(
        handle
            .send(AccountMessage::SessionInvalidated {
                reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
            })
            .await
    );
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (false, false, false, false)
    );
    while let Ok(actions) = action_rx.try_recv() {
        assert!(
            !matches!(
                actions.as_slice(),
                [AppAction::SessionAuthenticationInvalidated { .. }]
            ),
            "post-teardown invalidation must not dispatch a state action"
        );
    }
    assert!(
        !koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .any(|record| {
                record.event.source == "core.account" && record.event.stage == "session_invalidated"
            }),
        "post-teardown invalidation must not emit an admission diagnostic"
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn session_change_observer_stop_interrupts_blocked_mailbox_delivery() {
    let (tx, mut receiver) = mpsc::channel(1);
    tx.send(AccountMessage::Shutdown)
        .await
        .expect("fill the account mailbox");
    let (change_tx, change_rx) = broadcast::channel(1);
    let (stop_tx, stop_rx) = oneshot::channel();
    let delivery_barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut task = executor::spawn(run_session_change_observation(
        change_rx,
        tx,
        stop_rx,
        Some(delivery_barrier.clone()),
    ));
    let mut unknown_token = matrix_sdk::ruma::api::error::UnknownTokenErrorData::new();
    unknown_token.soft_logout = true;
    change_tx
        .send(matrix_sdk::SessionChange::UnknownToken(unknown_token))
        .expect("publish synthetic session invalidation");

    delivery_barrier.wait().await;
    stop_tx.send(()).expect("request observer stop");
    match executor::timeout(Duration::from_millis(250), &mut task).await {
        Ok(joined) => joined.expect("session-change observer task"),
        Err(_) => {
            task.abort();
            let _ = task.await;
            panic!("stop must interrupt a blocked session-change mailbox delivery");
        }
    }

    assert!(matches!(
        receiver.recv().await,
        Some(AccountMessage::Shutdown)
    ));
    assert!(
        receiver.try_recv().is_err(),
        "stop must discard only the blocked observer delivery"
    );
}

#[tokio::test]
async fn soft_logout_reauth_quiesces_old_runtime_before_installing_replacement() {
    let homeserver = spawn_quarantine_password_server();
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
    assert!(
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await
    );
    configure_verified_trust(&handle).await;
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: test_request_id(),
            request: LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    while probe_rx.try_recv().is_ok() {}

    assert!(
        handle
            .send(AccountMessage::SessionInvalidated {
                reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
            })
            .await
    );
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::SessionAuthenticationInvalidated { soft_logout: true }])
    ) {}

    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 2,
    };
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::SoftLogoutReauth {
                request_id,
                password: koushi_state::AuthSecret::new("synthetic-password"),
            }))
            .await
    );
    let mut installed = false;
    let mut succeeded = false;
    let mut trust_projection = None;
    while !installed || !succeeded {
        let actions = action_rx.recv().await.expect("reauth action channel");
        installed |= actions.iter().any(|action| {
            matches!(
                action,
                AppAction::SoftLogoutReauthSessionInstalled { request_id: 2, .. }
            )
        });
        succeeded |= actions.iter().any(|action| {
            matches!(
                action,
                AppAction::SoftLogoutReauthSucceeded { request_id: 2 }
            )
        });
        if let [
            AppAction::AuthoritativeDeviceTrustChanged {
                generation,
                transition_id,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            },
        ] = actions.as_slice()
        {
            trust_projection = Some((*generation, *transition_id));
        }
    }
    let _ = trust_projection;
    let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
    let sync_stopped = tokens
        .iter()
        .position(|token| *token == "shutdown_stop_sync_actor")
        .expect("the old sync owner must stop and join");
    let room_cleared = tokens
        .iter()
        .position(|token| *token == "shutdown_clear_room_session")
        .expect("the old room session must be cleared");
    let client_released = tokens
        .iter()
        .position(|token| *token == "locked_client_released")
        .expect("the invalid client must be released after child shutdown");
    assert!(sync_stopped < client_released, "{tokens:?}");
    assert!(room_cleared < client_released, "{tokens:?}");

    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn server_logout_best_effort_returns_on_timeout() {
    let outcome = wait_for_server_logout_best_effort(
        std::time::Duration::from_millis(1),
        futures_util::future::pending(),
    )
    .await;

    assert_eq!(outcome, ServerLogoutOutcome::TimedOut);
}

#[tokio::test]
async fn server_logout_best_effort_treats_network_failure_as_settled() {
    let outcome = wait_for_server_logout_best_effort(std::time::Duration::from_secs(1), async {
        Err(koushi_sdk::PasswordLoginError::Sdk(
            "synthetic network failure".to_owned(),
        ))
    })
    .await;

    assert_eq!(outcome, ServerLogoutOutcome::Failed);
}

/// Network-free: `RestoreLastSession` with no last-session pointer is the
/// NORMAL first-launch outcome — `SessionNotFound` failure event plus the
/// `RestoreSessionNotFound` projection so AppState shows SignedOut/login.
#[tokio::test]
async fn restore_last_session_without_pointer_emits_not_found() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession { request_id }
            ))
            .await
    );

    let actions = action_rx.recv().await.expect("reducer actions");
    assert!(
        matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
        "not-found must project RestoreSessionNotFound, got {actions:?}"
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

/// Network-free: a last-session pointer whose session data is gone (e.g.
/// cleared by logout) must follow the same not-found contract.
#[tokio::test]
async fn restore_last_session_with_dangling_pointer_emits_not_found() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");

    // Seed only the pointer — no session JSON behind it.
    let seeding_backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    let key_id = SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@dangling:example.test".to_owned(),
        device_id: "DEVICE1".to_owned(),
    };
    seeding_backend
        .save_last_session(&key_id)
        .expect("seed last-session pointer");

    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession { request_id }
            ))
            .await
    );

    let actions = action_rx.recv().await.expect("reducer actions");
    assert!(
        matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
        "dangling pointer must project RestoreSessionNotFound, got {actions:?}"
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

/// Network-free: `QuerySavedSessions` on an empty store answers with an
/// empty list — a normal outcome, not a failure.
#[tokio::test]
async fn query_saved_sessions_empty_store_lists_nothing() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::QuerySavedSessions { request_id }
            ))
            .await
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::Account(AccountEvent::SavedSessionsListed {
            request_id: ev_id,
            sessions,
        }) => {
            assert_eq!(ev_id, request_id);
            assert!(sessions.is_empty(), "expected empty list, got {sessions:?}");
        }
        other => panic!("expected SavedSessionsListed, got {other:?}"),
    }
}

/// Network-free: `QuerySavedSessions` lists seeded sessions with identity
/// data only (homeserver / user_id / device_id).
#[tokio::test]
async fn query_saved_sessions_lists_seeded_identities() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");

    let seeding_backend =
        CredentialStoreBackend::FileDir(koushi_store::FileCredentialStore::new(cred_dir.path()));
    let alpha = SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alpha:example.test".to_owned(),
        device_id: "DEVICE-A".to_owned(),
    };
    let beta = SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@beta:example.test".to_owned(),
        device_id: "DEVICE-B".to_owned(),
    };
    seeding_backend
        .remember_saved_session(&alpha)
        .expect("seed alpha");
    seeding_backend
        .remember_saved_session(&beta)
        .expect("seed beta");

    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::QuerySavedSessions { request_id }
            ))
            .await
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::Account(AccountEvent::SavedSessionsListed {
            request_id: ev_id,
            sessions,
        }) => {
            assert_eq!(ev_id, request_id);
            assert_eq!(sessions.len(), 2);
            assert!(
                sessions
                    .iter()
                    .any(|s| { s.user_id == "@alpha:example.test" && s.device_id == "DEVICE-A" })
            );
            assert!(
                sessions
                    .iter()
                    .any(|s| { s.user_id == "@beta:example.test" && s.device_id == "DEVICE-B" })
            );
            // Identity data only: SessionInfo has exactly homeserver /
            // user_id / device_id (enforced by type); the Debug output of
            // the event must not contain anything token-shaped.
            let debug = format!("{sessions:?}");
            assert!(!debug.contains("access_token"));
            assert!(!debug.contains("secret"));
        }
        other => panic!("expected SavedSessionsListed, got {other:?}"),
    }
}
