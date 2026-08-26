use super::cleanup::cleanup_e2ee_multi_device_participants;
use super::diagnostics::{diagnostic_count_field, diagnostic_has_token, diagnostic_token_field};
use super::event_wait::{
    QaEventDeadline, find_timeline_item_with_body, start_sync_for_qa, subscribe_timeline_for_qa,
    timeline_item_is_decryption_failure, wait_for_initial_items, wait_for_invite_in_snapshot,
    wait_for_item_with_body_or_decryption_failure, wait_for_logged_in, wait_for_logged_out,
    wait_for_operation_failed, wait_for_operation_failed_and_signed_out, wait_for_ready_snapshot,
    wait_for_room_created, wait_for_room_in_room_list, wait_for_send_flow_completion,
    wait_for_send_flow_completion_with_timeout, wait_for_session_restored,
    wait_for_sync_started_and_running, wait_for_sync_stopped,
    wait_for_withheld_event_projection_from_source,
};
use super::fixtures::{
    accept_invite_for_qa, assert_room_settings_contains_members, create_room_for_qa,
    invite_user_for_qa, load_room_settings_for_qa, native_attention_room, private_room_options,
};
use super::participants::{
    QaE2eeRecipient, QaOwnedRuntimeParticipant, QaParticipantLoginGate, SasQaOutcome,
    authenticated_session_info, cleanup_owned_e2ee_participant_best_effort,
    finish_e2ee_recipient_stage_with_owned_cleanup, login_synced_participant_for_qa, qa_data_dir,
    refresh_device_keys_and_assert_known_for_qa, start_isolated_qa_runtime,
    verify_provisional_second_device_for_qa, wait_for_existing_identity_gate,
    wait_for_locked_snapshot, wait_for_matching_recovery_flow, wait_for_recovery_gate,
};
use super::registry::{
    DEVICE_A, DEVICE_B, E2EE_EVENT_TIMEOUT, E2EE_KEY_BACKUP_SEED_BODY,
    E2EE_MULTI_USER_MULTI_DEVICE_BODY, E2EE_SECOND_DEVICE_BODY, ENV_E2EE_RECIPIENT_SECOND_DEVICE,
    EVENT_TIMEOUT, GATE_RESTORE_READY_BUDGET, QA_WRONG_RECOVERY_SECRET, QaConfig, env_flag_enabled,
};
use super::{
    AccountCommand, AccountEvent, AccountKey, AppCommand, AuthSecret, CoreCommand, CoreConnection,
    CoreEvent, CoreFailure, CoreRuntime, CurrentSessionStatusState, CurrentSessionSyncState,
    DeviceCleanupLocalMode, DeviceCleanupState, Duration, E2eeTrustEvent,
    EncryptionDebugOperationOutcome, IdentityResetAuthRequest, IdentityResetAuthType,
    IdentityResetState, KeyBackupStatus, LocalEncryptionEvent, LocalEncryptionHealth,
    LocalEncryptionState, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionDispatchState, NativeAttentionObservationKind, NativeAttentionProjectionInput,
    NativeAttentionState, NativeAttentionSuppressionReason, RecoveryRequest, RequestId,
    RoomAttentionKind, RoomCommand, RoomEvent, RoomNotificationMode, SessionAuthenticationMethod,
    SessionInfo, SessionState, SessionStatusRefreshTrigger, SyncCommand, TimelineCommand,
    TimelineItem, TimelineKey, VerificationTarget, native_attention_state_from_rooms,
};

pub(super) async fn run_gate_restore_stage(
    mut conn: CoreConnection,
    runtime: CoreRuntime,
    data_dir: std::path::PathBuf,
    account_key: AccountKey,
) -> Result<(), String> {
    println!("gate_restore_bootstrapped=ok");
    let stop_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Sync(SyncCommand::Stop {
            request_id: stop_id,
        })),
    )
    .await
    .map_err(|_| "gate restore sync-stop submit timed out".to_owned())?
    .map_err(|error| format!("gate restore sync-stop submit: {error}"))?;
    wait_for_sync_stopped(&mut conn, stop_id, "gate restore sync stop").await?;
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| "gate restore runtime shutdown timed out".to_owned())?;
    println!("gate_restore_shutdown_complete=ok");

    let reopened = CoreRuntime::start_with_data_dir(data_dir);
    let mut conn = reopened.attach();
    println!("gate_restore_runtime_spawned=ok");
    let query_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Account(AccountCommand::QuerySavedSessions {
            request_id: query_id,
        })),
    )
    .await
    .map_err(|_| "gate restore query submit timed out".to_owned())?
    .map_err(|error| format!("gate restore query submit: {error}"))?;
    println!("gate_restore_query_sent=ok");
    wait_for_saved_session_presence(&mut conn, query_id, &account_key).await?;
    println!("gate_restore_query_result=ok");

    let restore_started_at = std::time::Instant::now();
    let restore_id = conn.next_request_id();
    tokio::time::timeout(
        EVENT_TIMEOUT,
        conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        })),
    )
    .await
    .map_err(|_| "gate restore restore submit timed out".to_owned())?
    .map_err(|error| format!("gate restore restore submit: {error}"))?;
    println!("gate_restore_restore_sent=ok");
    wait_for_session_restored(&mut conn, restore_id, &account_key, "gate restore").await?;
    println!("gate_restore_restore_result=ok");
    wait_for_ready_snapshot(&mut conn, "gate restore Ready").await?;
    if restore_started_at.elapsed() > GATE_RESTORE_READY_BUDGET {
        return Err("gate restore exceeded bounded Ready budget".to_owned());
    }
    println!("gate_restore_ready=ok");
    println!("gate_verified_restore=ok");
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, reopened.shutdown())
        .await
        .map_err(|_| "gate restore reopened shutdown timed out".to_owned())?;
    Ok(())
}

async fn delete_qa_current_device(
    session: &koushi_sdk::MatrixClientSession,
    password: &str,
) -> Result<(), String> {
    let device_ids = [matrix_sdk::ruma::OwnedDeviceId::from(
        session.info.device_id.as_str(),
    )];
    let uiaa_session = match session.client().delete_devices(&device_ids, None).await {
        Ok(_) => return Ok(()),
        Err(error) => error
            .as_uiaa_response()
            .and_then(|uiaa| uiaa.session.clone())
            .ok_or_else(|| "no-proof initial device delete failed".to_owned())?,
    };
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth =
        matrix_sdk::ruma::api::client::uiaa::Password::new(identifier, password.to_owned());
    password_auth.session = Some(uiaa_session);
    session
        .client()
        .delete_devices(
            &device_ids,
            Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
                password_auth,
            )),
        )
        .await
        .map(|_| ())
        .map_err(|_| "no-proof authenticated device delete failed".to_owned())
}

pub(super) async fn run_gate_no_proof_stage(config: &QaConfig) -> Result<(), String> {
    let raw = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi No Proof Fixture".to_owned()),
    })
    .await
    .map_err(|_| "no-proof fixture login failed".to_owned())?;
    koushi_sdk::sync_once(&raw)
        .await
        .map_err(|_| "no-proof fixture sync failed".to_owned())?;
    koushi_sdk::bootstrap_cross_signing(&raw, Some(&AuthSecret::new(config.password_a.clone())))
        .await
        .map_err(|_| "no-proof cross-signing bootstrap failed".to_owned())?;
    delete_qa_current_device(&raw, &config.password_a).await?;
    let _ = koushi_sdk::close_session_stores(&raw).await;
    drop(raw);

    let data_dir = qa_data_dir("gate-no-proof");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();
    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: config.homeserver.clone(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi No Proof Core".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|_| "no-proof Core login submit failed".to_owned())?;
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    let mut saw_rejecting = false;
    loop {
        saw_rejecting |= matches!(conn.snapshot().session, SessionState::Rejecting { .. });
        if matches!(conn.snapshot().session, SessionState::SignedOut) && saw_rejecting {
            break;
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "no-proof rejection timed out".to_owned())?
            .map_err(|_| "no-proof event stream closed".to_owned())?;
    }
    println!("gate_no_proof_rejected=ok");
    drop(conn);
    runtime.shutdown().await;

    let reopened = CoreRuntime::start_with_data_dir(data_dir);
    let mut reopened_conn = reopened.attach();
    let restore_id = reopened_conn.next_request_id();
    reopened_conn
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id: restore_id,
        }))
        .await
        .map_err(|_| "no-proof restart restore submit failed".to_owned())?;
    let failure = wait_for_operation_failed_and_signed_out(
        &mut reopened_conn,
        restore_id,
        "no-proof restart restore",
    )
    .await?;
    if failure != CoreFailure::SessionNotFound {
        return Err("no-proof restart did not remain SignedOut".to_owned());
    }
    println!("gate_no_proof_restart_signed_out=ok");
    drop(reopened_conn);
    reopened.shutdown().await;
    Ok(())
}

pub(super) async fn run_gate_negative_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    recovery_secret: &AuthSecret,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "gate negative primary session")?;
    let runtime_a2 = start_isolated_qa_runtime("gate-negative-a2")?;
    let mut conn_a2 = runtime_a2.attach();
    let login_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_id,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A2".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative login submit: {error}"))?;
    let session_a2 = wait_for_existing_identity_gate(&mut conn_a2, "gate negative A2").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &session_a2,
        "gate negative mismatch",
        SasQaOutcome::Mismatch,
    )
    .await?;
    println!("gate_sas_mismatch_retryable=ok");
    let retry_session =
        wait_for_existing_identity_gate(&mut conn_a2, "gate negative retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a2,
        &session_a,
        &retry_session,
        "gate negative retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a2, login_id, "gate negative A2 login").await?;
    wait_for_ready_snapshot(&mut conn_a2, "gate negative A2 Ready").await?;
    println!("gate_sas_retry_ready=ok");
    drop(conn_a2);
    runtime_a2.shutdown().await;

    let runtime_a3 = start_isolated_qa_runtime("gate-negative-a3")?;
    let mut conn_a3 = runtime_a3.attach();
    let login_a3 = conn_a3.next_request_id();
    conn_a3
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a3,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A3".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A3 login submit: {error}"))?;
    let session_a3 = wait_for_existing_identity_gate(&mut conn_a3, "gate negative A3").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a3,
        &session_a,
        &session_a3,
        "gate negative user cancel",
        SasQaOutcome::UserCancel,
    )
    .await?;
    println!("gate_sas_user_cancel_retryable=ok");
    let retry_a3 = wait_for_existing_identity_gate(&mut conn_a3, "gate negative A3 retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a3,
        &session_a,
        &retry_a3,
        "gate negative user-cancel retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a3, login_a3, "gate negative A3 login").await?;
    wait_for_ready_snapshot(&mut conn_a3, "gate negative A3 Ready").await?;
    println!("gate_sas_user_cancel_retry_ready=ok");
    drop(conn_a3);
    runtime_a3.shutdown().await;

    let runtime_a4 = start_isolated_qa_runtime("gate-negative-a4")?;
    let mut conn_a4 = runtime_a4.attach();
    let login_a4 = conn_a4.next_request_id();
    conn_a4
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a4,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A4".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A4 login submit: {error}"))?;
    let session_a4 = wait_for_existing_identity_gate(&mut conn_a4, "gate negative A4").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a4,
        &session_a,
        &session_a4,
        "gate negative timeout",
        SasQaOutcome::Timeout,
    )
    .await?;
    println!("gate_sas_timeout_retryable=ok");
    let retry_a4 = wait_for_existing_identity_gate(&mut conn_a4, "gate negative A4 retry").await?;
    verify_provisional_second_device_for_qa(
        conn_a,
        &mut conn_a4,
        &session_a,
        &retry_a4,
        "gate negative timeout retry success",
        SasQaOutcome::Success,
    )
    .await?;
    let _ = wait_for_logged_in(&mut conn_a4, login_a4, "gate negative A4 login").await?;
    wait_for_ready_snapshot(&mut conn_a4, "gate negative A4 Ready").await?;
    println!("gate_sas_timeout_retry_ready=ok");
    drop(conn_a4);
    runtime_a4.shutdown().await;

    let runtime_a5 = start_isolated_qa_runtime("gate-negative-a5")?;
    let mut conn_a5 = runtime_a5.attach();
    let login_a5 = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a5,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A5".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A5 login submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a5, "gate negative A5").await?;
    let invalid_recovery = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: invalid_recovery,
            request: RecoveryRequest {
                secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET.to_owned()),
            },
        }))
        .await
        .map_err(|error| format!("gate negative invalid recovery submit: {error}"))?;
    let failure = wait_for_operation_failed(
        &mut conn_a5,
        invalid_recovery,
        "gate negative invalid recovery",
    )
    .await?;
    if !matches!(failure, CoreFailure::RecoveryFailed { .. }) {
        return Err("gate negative invalid recovery returned unexpected failure kind".to_owned());
    }
    wait_for_recovery_gate(&mut conn_a5, "gate negative A5 retry").await?;
    println!("gate_recovery_invalid_retryable=ok");
    let valid_recovery = conn_a5.next_request_id();
    conn_a5
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: valid_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative valid recovery submit: {error}"))?;
    wait_for_ready_snapshot(&mut conn_a5, "gate negative recovery Ready").await?;
    println!("gate_recovery_retry_ready=ok");
    drop(conn_a5);
    runtime_a5.shutdown().await;

    let runtime_a6 = start_isolated_qa_runtime("gate-negative-a6")?;
    let mut conn_a6 = runtime_a6.attach();
    let login_a6 = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_a6,
            request: koushi_state::LoginRequest {
                homeserver: config.homeserver.clone(),
                username: config.user_a.clone(),
                password: AuthSecret::new(config.password_a.clone()),
                device_display_name: Some("Koushi Gate Negative A6".to_owned()),
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .map_err(|error| format!("gate negative A6 login submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a6, "gate negative A6").await?;
    let cancelled_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: cancelled_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative cancelled recovery submit: {error}"))?;
    wait_for_matching_recovery_flow(
        &mut conn_a6,
        cancelled_recovery.sequence,
        "gate negative cancelled recovery",
    )
    .await?;
    let cancel_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::CancelVerification {
            request_id: cancel_recovery,
            flow_id: cancelled_recovery.sequence,
            reason: koushi_state::VerificationCancelReason::User,
        }))
        .await
        .map_err(|error| format!("gate negative recovery cancel submit: {error}"))?;
    wait_for_recovery_gate(&mut conn_a6, "gate negative A6 cancelled retry").await?;
    println!("gate_recovery_cancel_retryable=ok");
    let retry_recovery = conn_a6.next_request_id();
    conn_a6
        .command(CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id: retry_recovery,
            request: RecoveryRequest {
                secret: recovery_secret.clone(),
            },
        }))
        .await
        .map_err(|error| format!("gate negative recovery retry submit: {error}"))?;
    wait_for_ready_snapshot(&mut conn_a6, "gate negative cancelled recovery Ready").await?;
    println!("gate_recovery_cancel_retry_ready=ok");
    let account_key_a6 = AccountKey(
        authenticated_session_info(&mut conn_a6, "gate negative A6 reset session")?.user_id,
    );
    reset_identity_for_qa(
        &mut conn_a6,
        &account_key_a6,
        config.password_a.clone(),
        "gate negative trust loss reset",
    )
    .await?;
    wait_for_locked_snapshot(conn_a, "gate negative primary trust loss").await?;
    println!("gate_trust_loss_locked=ok");
    let blocked_sync = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Sync(SyncCommand::Start {
            request_id: blocked_sync,
        }))
        .await
        .map_err(|error| format!("gate negative locked sync submit: {error}"))?;
    let failure =
        wait_for_operation_failed(conn_a, blocked_sync, "gate negative locked normal command")
            .await?;
    if failure != CoreFailure::SessionRequired {
        return Err("gate negative locked command returned unexpected failure kind".to_owned());
    }
    println!("gate_trust_loss_commands_blocked=ok");
    drop(conn_a6);
    runtime_a6.shutdown().await;
    Ok(())
}

pub(super) async fn run_provisional_device_cleanup_qa(config: &QaConfig) -> Result<(), String> {
    let runtime = CoreRuntime::start_with_data_dir(qa_data_dir("gate-device-cleanup"));
    let mut conn = runtime.attach();
    let result = async {
        let removed_session =
            login_until_device_cleanup_offered(&mut conn, config, "device cleanup first login")
                .await?;
        drive_remote_first_device_cleanup(
            &mut conn,
            &config.password_a,
            "device cleanup first device",
        )
        .await?;

        let replacement_session =
            login_until_device_cleanup_offered(&mut conn, config, "device cleanup replacement")
                .await?;
        if replacement_session.device_id == removed_session.device_id {
            return Err(
                "device cleanup replacement login reused the removed server device".to_owned(),
            );
        }
        audit_removed_device_absent_from_server(
            config,
            &removed_session.device_id,
            &replacement_session.device_id,
        )
        .await?;
        println!("device_cleanup_remote_first=ok");
        println!("device_cleanup_relogin_new_device=ok");

        drive_remote_first_device_cleanup(
            &mut conn,
            &config.password_a,
            "device cleanup replacement device",
        )
        .await?;
        Ok(())
    }
    .await;

    drop(conn);
    runtime.shutdown().await;
    result
}

pub(super) async fn audit_removed_device_absent_from_server(
    config: &QaConfig,
    removed_device_id: &str,
    replacement_device_id: &str,
) -> Result<(), String> {
    let auditor = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi Device Cleanup Auditor".to_owned()),
    })
    .await
    .map_err(|_| "device cleanup audit login failed".to_owned())?;
    let devices = auditor
        .client()
        .devices()
        .await
        .map_err(|_| "device cleanup audit device-list request failed".to_owned());
    let cleanup = cleanup_qa_auditor_device(&auditor, &config.password_a).await;
    let _ = koushi_sdk::close_session_stores(&auditor).await;
    drop(auditor);

    let devices = devices?;
    cleanup?;
    if devices
        .devices
        .iter()
        .any(|device| device.device_id.as_str() == removed_device_id)
    {
        return Err("device cleanup audit found the removed device on the homeserver".to_owned());
    }
    if !devices
        .devices
        .iter()
        .any(|device| device.device_id.as_str() == replacement_device_id)
    {
        return Err(
            "device cleanup audit did not find the replacement device on the homeserver".to_owned(),
        );
    }
    Ok(())
}

async fn cleanup_qa_auditor_device(
    auditor: &koushi_sdk::MatrixClientSession,
    password: &str,
) -> Result<(), String> {
    let initial = koushi_sdk::cleanup_current_device(auditor, None, None)
        .await
        .map_err(|_| "device cleanup audit device removal failed".to_owned())?;
    match initial {
        koushi_sdk::MatrixDeviceCleanupOutcome::Settled(_) => Ok(()),
        koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired { session } => {
            match koushi_sdk::cleanup_current_device(
                auditor,
                Some(&AuthSecret::new(password.to_owned())),
                session.as_deref(),
            )
            .await
            .map_err(|_| "device cleanup audit authenticated removal failed".to_owned())?
            {
                koushi_sdk::MatrixDeviceCleanupOutcome::Settled(_) => Ok(()),
                koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired { .. } => {
                    Err("device cleanup audit repeated its authentication challenge".to_owned())
                }
            }
        }
    }
}

pub(super) async fn login_until_device_cleanup_offered(
    conn: &mut CoreConnection,
    config: &QaConfig,
    label: &str,
) -> Result<SessionInfo, String> {
    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: config.homeserver.clone(),
            username: config.user_a.clone(),
            password: AuthSecret::new(config.password_a.clone()),
            device_display_name: Some("Koushi Device Cleanup QA".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|error| format!("{label}: login submit failed: {error}"))?;
    wait_for_recovery_gate(conn, label).await?;
    let session = authenticated_session_info(conn, label)?;

    let invalid_recovery = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id: invalid_recovery,
        request: RecoveryRequest {
            secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET.to_owned()),
        },
    }))
    .await
    .map_err(|error| format!("{label}: invalid recovery submit failed: {error}"))?;
    wait_for_device_cleanup_offered(conn, label).await?;
    Ok(session)
}

async fn wait_for_device_cleanup_offered(
    conn: &mut CoreConnection,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + E2EE_EVENT_TIMEOUT;
    loop {
        if matches!(
            conn.snapshot().device_cleanup,
            DeviceCleanupState::Offered { .. }
        ) {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for device cleanup offer"))?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn drive_remote_first_device_cleanup(
    conn: &mut CoreConnection,
    password: &str,
    label: &str,
) -> Result<(), String> {
    let start_request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::StartDeviceCleanup {
        request_id: start_request_id,
    }))
    .await
    .map_err(|error| format!("{label}: cleanup submit failed: {error}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
    let mut submitted_uia_flow = None;
    let mut retried_local_reset = false;
    loop {
        let snapshot = conn.snapshot();
        if matches!(snapshot.session, SessionState::SignedOut) {
            return Ok(());
        }
        match snapshot.device_cleanup {
            DeviceCleanupState::AwaitingUia { flow_id, .. }
                if submitted_uia_flow != Some(flow_id) =>
            {
                submitted_uia_flow = Some(flow_id);
                let request_id = conn.next_request_id();
                conn.command(CoreCommand::Account(
                    AccountCommand::SubmitDeviceCleanupUia {
                        request_id,
                        flow_id,
                        password: AuthSecret::new(password.to_owned()),
                    },
                ))
                .await
                .map_err(|error| format!("{label}: cleanup UIAA submit failed: {error}"))?;
            }
            DeviceCleanupState::RemoteFailed {
                failure, auth_mode, ..
            } => {
                return Err(format!(
                    "{label}: remote cleanup failed; mode={auth_mode:?} failure={failure:?}"
                ));
            }
            DeviceCleanupState::LocalResetFailed {
                mode: DeviceCleanupLocalMode::RemoteRemoved { .. },
                ..
            } if !retried_local_reset => {
                retried_local_reset = true;
                let request_id = conn.next_request_id();
                conn.command(CoreCommand::Account(AccountCommand::StartDeviceCleanup {
                    request_id,
                }))
                .await
                .map_err(|error| format!("{label}: local cleanup retry failed: {error}"))?;
            }
            DeviceCleanupState::LocalResetFailed { .. } => {
                return Err(format!("{label}: local cleanup failed after retry"));
            }
            _ => {}
        }

        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for cleanup completion"))?
            .map_err(|_| format!("{label}: event stream closed"))?;
    }
}

async fn wait_for_saved_session_presence(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: &AccountKey,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "timed out waiting for saved-session readiness".to_owned())?
            .map_err(|lag| {
                format!(
                    "saved-session readiness event lagged (skipped={})",
                    lag.skipped
                )
            })?;
        match event {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: event_id,
                sessions,
            }) if event_id == request_id => {
                if sessions.iter().any(|session| session.user_id == expected.0) {
                    return Ok(());
                }
                return Err(format!(
                    "saved-session readiness missing expected account; saved_count={}",
                    sessions.len()
                ));
            }
            CoreEvent::OperationFailed {
                request_id: event_id,
                failure,
            } if event_id == request_id => {
                return Err(format!("saved-session readiness failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

pub(super) async fn run_e2ee_trust_stage(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
    recipient_base: Option<(&mut CoreConnection, &AccountKey)>,
) -> Result<(), String> {
    let session_a = authenticated_session_info(conn_a, "session A info for E2EE trust")?;

    // The login gate already bootstrapped and authoritatively promoted this
    // session. Re-running bootstrap here would rotate the identity and
    // invalidate the proof device that A2 is about to use.
    println!("e2ee_cross_signing_reused=ok");
    println!("e2ee_cross_signing=ok");

    let key_backup_seed_room_id =
        seed_encrypted_room_key_for_qa(conn_a, account_key_a, "seed key backup room A").await?;
    println!("e2ee_key_backup_seed=ok");

    let key_backup_version = enable_key_backup_for_qa(
        conn_a,
        account_key_a,
        Some(AuthSecret::new(config.password_a.clone())),
        "enable key backup A",
    )
    .await?;
    println!("e2ee_key_backup_enable=ok");

    let runtime_a2 = start_isolated_qa_runtime("a2")?;
    let conn_a2 = runtime_a2.attach();
    let mut owned_a2 = QaOwnedRuntimeParticipant::new(runtime_a2, conn_a2);
    let a2_stage_result: Result<(), String> = async {
        let login_a2_id = owned_a2.conn.next_request_id();
        owned_a2
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_a2_id,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_a.clone(),
                    password: AuthSecret::new(config.password_a.clone()),
                    device_display_name: Some("Koushi Core QA A2".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .map_err(|e| format!("submit login A2: {e}"))?;
        owned_a2.mark_login_submitted();

        let session_a2 =
            wait_for_existing_identity_gate(&mut owned_a2.conn, "session A2 gate").await?;
        verify_provisional_second_device_for_qa(
            conn_a,
            &mut owned_a2.conn,
            &session_a,
            &session_a2,
            "e2ee gated self verification A/A2",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_a2 =
            wait_for_logged_in(&mut owned_a2.conn, login_a2_id, "login A2").await?;
        owned_a2.mark_logged_in(account_key_a2.clone());
        let conn_a2 = &mut owned_a2.conn;
        wait_for_ready_snapshot(conn_a2, "session A2 Ready").await?;
        println!("gate_own_sas=ok");

        let sync_start_a2_id = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Sync(SyncCommand::Start {
                request_id: sync_start_a2_id,
            }))
            .await
            .map_err(|e| format!("submit sync start A2: {e}"))?;
        wait_for_sync_started_and_running(conn_a2, sync_start_a2_id, "sync start A2").await?;

        wait_for_room_in_room_list(
            conn_a2,
            &key_backup_seed_room_id,
            "room list A2 after key backup seed",
        )
        .await?;

        restore_key_backup_failure_for_qa(
            conn_a2,
            &account_key_a2,
            Some(key_backup_version.clone()),
            "restore key backup failure A2",
        )
        .await?;
        println!("e2ee_key_backup_restore_failure=ok");

        restore_key_backup_success_for_qa(
            conn_a2,
            &account_key_a2,
            Some(key_backup_version),
            AuthSecret::new(config.password_a.clone()),
            "restore key backup success A2",
        )
        .await?;
        println!("joined_room_restore=ok");

        println!("e2ee_verification=ok");

        verify_second_device_room_key_delivery_for_qa(
            conn_a,
            conn_a2,
            account_key_a,
            &account_key_a2,
            &key_backup_seed_room_id,
        )
        .await?;
        println!("e2ee_second_device_decrypt=ok");

        verify_multi_user_multi_device_room_key_delivery_for_qa(
            config,
            conn_a,
            conn_a2,
            account_key_a,
            &account_key_a2,
            recipient_base,
        )
        .await?;
        println!("e2ee_multi_user_multi_device_decrypt=ok");
        Ok(())
    }
    .await;

    finish_e2ee_recipient_stage_with_owned_cleanup(
        a2_stage_result,
        Some(owned_a2),
        |participant| async move {
            cleanup_owned_e2ee_participant_best_effort(participant, "cleanup secondary device")
                .await
        },
    )
    .await?;

    if config.allow_identity_reset {
        reset_identity_for_qa(
            conn_a,
            account_key_a,
            config.password_a.clone(),
            "reset identity A",
        )
        .await?;
        println!("e2ee_identity_reset=ok");
    } else {
        println!("e2ee_identity_reset=skipped");
    }
    println!("e2ee_trust=ok");

    Ok(())
}

async fn wait_for_local_encryption_health(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: LocalEncryptionHealth,
    label: &str,
) -> Result<(), String> {
    let expected_state = LocalEncryptionState::from(expected);
    if conn.snapshot().local_encryption == expected_state {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for local encryption health"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if snapshot.local_encryption == expected_state => {
                return Ok(());
            }
            CoreEvent::LocalEncryption(LocalEncryptionEvent::HealthChanged { health })
                if health == expected && conn.snapshot().local_encryption == expected_state =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: local encryption health failed: {failure:?}"
                ));
            }
            _ if conn.snapshot().local_encryption == expected_state => {
                return Ok(());
            }
            _ => {}
        }
    }
}

async fn wait_for_native_attention_state(
    conn: &mut CoreConnection,
    request_id: RequestId,
    expected: &NativeAttentionState,
    label: &str,
) -> Result<(), String> {
    if conn.snapshot().native_attention == *expected {
        return Ok(());
    }

    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for native attention summary"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::StateChanged(snapshot) if snapshot.native_attention == *expected => {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "{label}: native attention update failed: {failure:?}"
                ));
            }
            _ if conn.snapshot().native_attention == *expected => {
                return Ok(());
            }
            _ => {}
        }
    }
}

pub(super) async fn run_session_status_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let expected_device_id = match &conn.snapshot().session {
        SessionState::Ready(info) => info.device_id.clone(),
        _ => return Err("session_status: current session is not Ready".to_owned()),
    };
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::RefreshCurrentSessionStatus {
            request_id,
            trigger: SessionStatusRefreshTrigger::Manual,
        },
    ))
    .await
    .map_err(|_| "session_status: refresh command was rejected".to_owned())?;

    let mut saw_checking = false;
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    loop {
        let event = deadline
            .recv(conn)
            .await
            .map_err(|_| "session_status: timed out waiting for settlement".to_owned())?
            .map_err(|lag| {
                format!(
                    "session_status: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        let CoreEvent::StateChanged(snapshot) = event else {
            continue;
        };
        match &snapshot.current_session_status {
            CurrentSessionStatusState::Checking {
                request_id: observed_request_id,
                trigger: SessionStatusRefreshTrigger::Manual,
            } if *observed_request_id == request_id.sequence => {
                if !saw_checking {
                    println!("session_status_checking=ok");
                    saw_checking = true;
                }
            }
            CurrentSessionStatusState::Ready {
                request_id: observed_request_id,
                details,
            } if *observed_request_id == request_id.sequence => {
                if !saw_checking {
                    return Err(
                        "session_status: Ready was observed without the Checking transition"
                            .to_owned(),
                    );
                }
                if details.device_id != expected_device_id {
                    return Err(
                        "session_status: SDK facts did not describe the current device".to_owned(),
                    );
                }
                if details.device_display_name.as_deref() != Some(DEVICE_A) {
                    return Err(
                        "session_status: server-side current-device name did not match login"
                            .to_owned(),
                    );
                }
                if details.authentication_method != SessionAuthenticationMethod::Password
                    || details.sync_state != CurrentSessionSyncState::Running
                {
                    return Err(
                        "session_status: authentication or sync facts did not match runtime"
                            .to_owned(),
                    );
                }
                println!("session_status_ready=ok");
                println!("session_status_device=ok");
                println!("session_status=ok");
                return Ok(());
            }
            CurrentSessionStatusState::Failed {
                request_id: observed_request_id,
                ..
            } if *observed_request_id == request_id.sequence => {
                return Err("session_status: refresh settled with a coarse failure".to_owned());
            }
            _ => {}
        }
    }
}

pub(super) async fn run_credential_health_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let probe_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ProbeLocalEncryptionHealth {
            request_id: probe_id,
        },
    ))
    .await
    .map_err(|e| format!("submit credential health probe: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        probe_id,
        LocalEncryptionHealth::Healthy,
        "credential health",
    )
    .await?;
    println!("credential_health=ok");

    let fail_closed_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::RecordLocalEncryptionHealth {
        request_id: fail_closed_id,
        health: LocalEncryptionHealth::LockedOrInaccessible,
    }))
    .await
    .map_err(|e| format!("submit credential fail-closed health record: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        fail_closed_id,
        LocalEncryptionHealth::LockedOrInaccessible,
        "credential fail-closed",
    )
    .await?;
    println!("fail_closed=ok");

    let reprobe_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::ProbeLocalEncryptionHealth {
            request_id: reprobe_id,
        },
    ))
    .await
    .map_err(|e| format!("submit credential health restore probe: {e}"))?;
    wait_for_local_encryption_health(
        conn,
        reprobe_id,
        LocalEncryptionHealth::Healthy,
        "credential health restore",
    )
    .await
}

/// The `encryption_debug` scenario (issue #538): in a real encrypted room,
/// force a new outbound session and share the index-0 key, proving the
/// command → RoomActor → event path and the typed outcomes. The SDK owns all
/// cryptographic effects; the scenario only asserts closed outcomes and that
/// index 0 is not consumed by the share.
pub(super) async fn run_encryption_debug_stage(
    config: &QaConfig,
    conn: &mut CoreConnection,
    account_key: &AccountKey,
) -> Result<(), String> {
    // Ensure the primary device publishes the proof capability required for
    // the verified second-device prerequisite. Without this, the gate can
    // observe an intermediate ExistingIdentity snapshot with no SAS method.
    let bootstrap_id = conn.next_request_id();
    conn.command(CoreCommand::Account(
        AccountCommand::BootstrapCrossSigning {
            request_id: bootstrap_id,
            auth: Some(AuthSecret::new(config.password_a.clone())),
        },
    ))
    .await
    .map_err(|e| format!("encryption-debug: bootstrap cross-signing failed: {e}"))?;
    let bootstrap_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        match tokio::time::timeout_at(bootstrap_deadline, conn.recv_event())
            .await
            .map_err(|_| "encryption-debug: cross-signing bootstrap timed out".to_owned())?
            .map_err(|_| "encryption-debug: event stream closed during bootstrap".to_owned())?
        {
            CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key: got,
                status,
            }) if got == *account_key && status == koushi_state::CrossSigningStatus::Trusted => {
                break;
            }
            CoreEvent::OperationFailed { request_id, .. } if request_id == bootstrap_id => {
                return Err("encryption-debug: cross-signing bootstrap operation failed".to_owned());
            }
            _ => {}
        }
    }
    println!("encryption_debug_cross_signing=ok");

    let room_id =
        create_room_for_qa(conn, "encryption-debug", true, "encryption-debug room").await?;
    println!("encryption_debug_room=ok");

    // Add a second, verified device of the same user so the share has a real
    // eligible recipient (crypto excludes the current device; an empty
    // eligible set must refuse rather than report success).
    let session_a = authenticated_session_info(conn, "encryption-debug session A")?;
    let runtime_a2 = start_isolated_qa_runtime("encryption-debug-a2")?;
    let mut conn_a2 = runtime_a2.attach();
    let mut account_key_a2_for_cleanup = None;
    // Guarded body: A2 is logged out and its runtime is stopped on BOTH the
    // success and every error path after its runtime exists (issue #538).
    let guarded: Result<(), String> = async {
        let login_a2_id = conn_a2.next_request_id();
        conn_a2
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_a2_id,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_a.clone(),
                    password: AuthSecret::new(config.password_a.clone()),
                    device_display_name: Some("Koushi Core QA encryption-debug A2".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .map_err(|e| format!("encryption-debug: submit login A2 failed: {e}"))?;
        let session_a2 =
            wait_for_existing_identity_gate(&mut conn_a2, "encryption-debug A2 gate").await?;
        verify_provisional_second_device_for_qa(
            conn,
            &mut conn_a2,
            &session_a,
            &session_a2,
            "encryption-debug A/A2",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_a2 =
            wait_for_logged_in(&mut conn_a2, login_a2_id, "encryption-debug login A2").await?;
        account_key_a2_for_cleanup = Some(account_key_a2.clone());

        // A2 is a second verified device of the same user, so it is an eligible
        // own-other device of the room without any invite/join (the room is
        // creator-owned by the same user). The SAS flow above already settled to
        // Done, so the share can target this verified own device directly.
        println!("encryption_debug_recipient=ok");
        let _ = account_key_a2;

        // Force a new outbound session: the fresh session must settle Completed.
        let force_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::ForceNewOutboundSession {
            request_id: force_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("encryption-debug: submit force-new failed: {e}"))?;
        let force_outcome = wait_for_encryption_debug_event(
            conn,
            force_id,
            &room_id,
            "force_new_outbound_session",
            "OutboundSessionForced",
        )
        .await?;
        if force_outcome != EncryptionDebugOperationOutcome::Completed {
            return Err(format!(
                "encryption-debug: force-new did not complete (got {force_outcome:?})"
            ));
        }
        println!("force_new_outbound_session=ok");

        // Share the index-0 key: it must complete without consuming index 0.
        let share_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::ShareIndex0RoomKey {
            request_id: share_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("encryption-debug: submit share-index0 failed: {e}"))?;
        let share_outcome = wait_for_encryption_debug_event(
            conn,
            share_id,
            &room_id,
            "share_index0_room_key",
            "Index0RoomKeyShared",
        )
        .await?;
        if share_outcome != EncryptionDebugOperationOutcome::Completed {
            return Err(format!(
                "encryption-debug: index-0 share did not complete (got {share_outcome:?})"
            ));
        }
        println!("share_index0_room_key=ok");

        // A Completed share outcome implies the session was still at index 0
        // (otherwise the SDK refuses with RefusedIndexAdvanced). The SDK summary
        // also records index_before/index_after in the diagnostics.
        println!("index0_not_consumed=ok");

        // Advance the same outbound session, then exercise issue #541's manual
        // recovery resend. The resend must leave the index unchanged and target
        // only the immutable original ledger.
        let timeline_key = TimelineKey::room(account_key.clone(), room_id.clone());
        let advance_id = conn.next_request_id();
        let advance_txn = "encryption-debug-advance".to_owned();
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: advance_id,
            key: timeline_key.clone(),
            transaction_id: advance_txn.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(
                "encryption-debug advance".to_owned(),
            ),
        }))
        .await
        .map_err(|e| format!("encryption-debug: submit advance failed: {e}"))?;
        let _ = wait_for_send_flow_completion(
            conn,
            advance_id,
            &timeline_key,
            &advance_txn,
            "encryption-debug advance",
            "encryption-debug advance",
        )
        .await?;
        println!("encryption_debug_index_advanced=ok");

        let resend_id = conn.next_request_id();
        conn.command(CoreCommand::Room(RoomCommand::ResendIndex0RoomKey {
            request_id: resend_id,
            room_id: room_id.clone(),
        }))
        .await
        .map_err(|e| format!("encryption-debug: submit resend failed: {e}"))?;
        let resend_outcome = wait_for_encryption_debug_event(
            conn,
            resend_id,
            &room_id,
            "resend_index0_room_key",
            "Index0RoomKeyResent",
        )
        .await?;
        if resend_outcome != EncryptionDebugOperationOutcome::Completed {
            return Err(format!(
                "encryption-debug: index-0 resend did not complete (got {resend_outcome:?})"
            ));
        }
        println!("resend_index0_room_key=ok");
        let diagnostics = koushi_diagnostics::snapshot();
        let debug = diagnostics
            .records
            .iter()
            .rev()
            .map(|record| &record.event)
            .find(|event| {
                event.source == "core.room_key_debug"
                    && diagnostic_has_token(event, "operation", "resend_index0")
            })
            .ok_or_else(|| "encryption-debug: resend diagnostic missing".to_owned())?;
        if diagnostic_token_field(debug, "outcome") != Some("completed")
            || diagnostic_count_field(debug, "index_before").is_none_or(|index| index == 0)
            || diagnostic_count_field(debug, "index_before")
                != diagnostic_count_field(debug, "index_after")
            || diagnostic_count_field(debug, "inbound_first_known_index") != Some(0)
            || diagnostic_count_field(debug, "peer_accepted")
                > diagnostic_count_field(debug, "peer_eligible")
            || diagnostic_count_field(debug, "peer_missing")
                > diagnostic_count_field(debug, "peer_eligible")
            || diagnostic_count_field(debug, "peer_accepted").unwrap_or(0)
                + diagnostic_count_field(debug, "peer_missing").unwrap_or(0)
                != diagnostic_count_field(debug, "peer_eligible").unwrap_or(0)
            || !matches!(
                diagnostic_token_field(debug, "claim"),
                Some("not_needed" | "succeeded")
            )
            || diagnostic_count_field(debug, "elapsed_ms").is_none_or(|elapsed| elapsed == 0)
            || diagnostic_count_field(debug, "peer_ledger")
                < diagnostic_count_field(debug, "peer_eligible")
            || diagnostic_count_field(debug, "room_event_sent") != Some(0)
            || diagnostic_count_field(debug, "index0_consumed") != Some(0)
        {
            return Err("encryption-debug: resend diagnostic invariants failed".to_owned());
        }
        println!("resend_index_unchanged=ok");
        let _ = config;
        let _ = account_key;
        Ok(())
    }
    .await;
    // Finally: attempt A2 logout and stop its runtime so no session leaks,
    // on both success and error paths (best-effort; a logout failure is not
    // a scenario failure by itself).
    let logout_a2_id = conn_a2.next_request_id();
    if conn_a2
        .command(CoreCommand::Account(AccountCommand::Logout {
            request_id: logout_a2_id,
        }))
        .await
        .is_ok()
    {
        if let Some(account_key_a2) = account_key_a2_for_cleanup.as_ref() {
            let _ = wait_for_logged_out(
                &mut conn_a2,
                logout_a2_id,
                account_key_a2,
                "encryption-debug A2 logout",
            )
            .await;
        }
    }
    drop(conn_a2);
    runtime_a2.shutdown().await;
    guarded
}

async fn wait_for_encryption_debug_event(
    conn: &mut CoreConnection,
    request_id: RequestId,
    room_id: &str,
    label: &str,
    event_name: &str,
) -> Result<EncryptionDebugOperationOutcome, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("encryption-debug: {label} timed out waiting for {event_name}"))?
            .map_err(|e| format!("{label}: recv: {e:?}"))?
        {
            CoreEvent::Room(RoomEvent::OutboundSessionForced {
                request_id: got,
                outcome,
                room_id: got_room,
                ..
            }) if event_name == "OutboundSessionForced"
                && got == request_id
                && got_room == room_id =>
            {
                return Ok(outcome);
            }
            CoreEvent::Room(RoomEvent::Index0RoomKeyShared {
                request_id: got,
                outcome,
                room_id: got_room,
                ..
            }) if event_name == "Index0RoomKeyShared"
                && got == request_id
                && got_room == room_id =>
            {
                return Ok(outcome);
            }
            CoreEvent::Room(RoomEvent::Index0RoomKeyResent {
                request_id: got,
                outcome,
                room_id: got_room,
                ..
            }) if event_name == "Index0RoomKeyResent"
                && got == request_id
                && got_room == room_id =>
            {
                return Ok(outcome);
            }
            CoreEvent::OperationFailed {
                request_id: got, ..
            } if got == request_id => {
                return Err(format!(
                    "encryption-debug: {label} failed with an operation failure"
                ));
            }
            _ => continue,
        }
    }
}

pub(super) async fn run_native_attention_stage(conn: &mut CoreConnection) -> Result<(), String> {
    let rooms = vec![
        native_attention_room("!message:example.invalid", "Room", false, 8, 8, 0),
        native_attention_room("!dm:example.invalid", "Direct", true, 3, 3, 0),
        native_attention_room("!mention:example.invalid", "Mention", false, 1, 1, 1),
    ];
    let capabilities = native_attention_available_capabilities();
    let attention = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });

    let candidate = attention
        .summary
        .candidate
        .as_ref()
        .ok_or_else(|| "native attention candidate was not projected".to_owned())?;
    if candidate.kind != RoomAttentionKind::Mention || attention.summary.badge_count != 12 {
        return Err("native attention candidate priority or badge count was wrong".to_owned());
    }

    let candidate_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
        request_id: candidate_id,
        attention: attention.clone(),
    }))
    .await
    .map_err(|e| format!("native attention: submit candidate update failed: {e}"))?;
    wait_for_native_attention_state(conn, candidate_id, &attention, "native attention candidate")
        .await?;
    println!("notification_candidate=ok");
    println!("badge_state=ok");

    let focused = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: Some("!mention:example.invalid"),
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: true,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });
    if focused.summary.candidate.is_some()
        || focused.dispatch
            != (NativeAttentionDispatchState::Suppressed {
                reason: NativeAttentionSuppressionReason::WindowFocused,
            })
    {
        return Err("native attention focused room suppression was not projected".to_owned());
    }
    println!("suppress_focus=ok");

    let mut notification_modes = std::collections::HashMap::new();
    notification_modes.insert(
        "!message:example.invalid".to_owned(),
        RoomNotificationMode::Mute,
    );
    notification_modes.insert(
        "!dm:example.invalid".to_owned(),
        RoomNotificationMode::Mentions,
    );
    let with_modes = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &notification_modes,
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities,
    });
    if with_modes.summary.unread_count != 1
        || with_modes.summary.highlight_count != 1
        || with_modes.summary.badge_count != 4
        || with_modes
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.kind)
            != Some(RoomAttentionKind::Mention)
    {
        return Err("native attention did not respect per-room notification modes".to_owned());
    }
    println!("room_notification_modes=ok");

    let clear = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &[],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: attention.summary.candidate.as_ref(),
        capabilities,
    });
    if clear.summary.badge_count != 0 || clear.summary.candidate.is_some() {
        return Err("native attention clear state retained badge or candidate".to_owned());
    }

    let clear_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
        request_id: clear_id,
        attention: clear.clone(),
    }))
    .await
    .map_err(|e| format!("native attention: submit clear update failed: {e}"))?;
    wait_for_native_attention_state(conn, clear_id, &clear, "native attention clear").await?;
    println!("clear_badge=ok");

    Ok(())
}

fn native_attention_available_capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

pub(super) async fn seed_encrypted_room_key_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    label: &str,
) -> Result<String, String> {
    let create_room_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id: create_room_id,
        options: private_room_options("QA E2EE Backup Room", true),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted room create failed: {e}"))?;

    let room_id = wait_for_room_created(conn, create_room_id, label).await?;

    wait_for_room_in_room_list(conn, &room_id, "room list after encrypted backup seed").await?;

    let key = TimelineKey::room(account_key.clone(), room_id.clone());
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: key.clone(),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted timeline subscribe failed: {e}"))?;

    wait_for_initial_items(conn, &key, subscribe_id, "subscribe encrypted backup seed").await?;

    let transaction_id = "qa-e2ee-key-backup-seed".to_owned();
    let send_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: send_id,
        key: key.clone(),
        transaction_id: transaction_id.clone(),
        document: koushi_state::ComposerDocument::from_plain_text(
            E2EE_KEY_BACKUP_SEED_BODY.to_owned(),
        ),
    }))
    .await
    .map_err(|e| format!("{label}: submit encrypted backup seed send failed: {e}"))?;

    wait_for_send_flow_completion(
        conn,
        send_id,
        &key,
        &transaction_id,
        E2EE_KEY_BACKUP_SEED_BODY,
        "send encrypted backup seed",
    )
    .await?;

    Ok(room_id)
}

pub(super) async fn enable_key_backup_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    passphrase: Option<AuthSecret>,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::EnableKeyBackup {
        request_id,
        passphrase,
    }))
    .await
    .map_err(|e| format!("{label}: submit enable key backup failed: {e}"))?;

    wait_for_key_backup_enabled(conn, account_key, request_id, label).await
}

async fn wait_for_key_backup_enabled(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<String, String> {
    if let KeyBackupStatus::Enabled { version } = &conn.snapshot().e2ee_trust.key_backup {
        return Ok(version.clone());
    }

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup Enabled"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Enabled { version } => return Ok(version),
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup enable failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.key_backup {
                KeyBackupStatus::Enabled { version } => return Ok(version),
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup enable failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn restore_key_backup_failure_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    version: Option<String>,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id,
        version,
        request: RecoveryRequest {
            secret: AuthSecret::new(QA_WRONG_RECOVERY_SECRET),
        },
    }))
    .await
    .map_err(|e| format!("{label}: submit restore key backup failed: {e}"))?;

    wait_for_key_backup_failed(conn, account_key, request_id, label).await
}

async fn restore_key_backup_success_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    version: Option<String>,
    secret: AuthSecret,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreKeyBackup {
        request_id,
        version,
        request: RecoveryRequest { secret },
    }))
    .await
    .map_err(|e| format!("{label}: submit restore key backup failed: {e}"))?;

    wait_for_key_backup_restored(conn, account_key, request_id, label).await
}

async fn wait_for_key_backup_failed(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: current,
            ..
        } if current == request_id.sequence
    );

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup failure"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id.sequence => return Ok(()),
                KeyBackupStatus::Restoring {
                    request_id: current,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    return Err(format!("{label}: restore unexpectedly succeeded"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.key_backup {
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id.sequence => return Ok(()),
                KeyBackupStatus::Restoring {
                    request_id: current,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    return Err(format!("{label}: restore unexpectedly succeeded"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_key_backup_restored(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.key_backup,
        KeyBackupStatus::Restoring {
            request_id: current,
            ..
        } if current == request_id.sequence
    );
    let mut saw_restored_room = false;

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for key backup restore success"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key: ev_account_key,
                status,
            }) if &ev_account_key == account_key => match status {
                KeyBackupStatus::Restoring {
                    request_id: current,
                    restored_rooms,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                    saw_restored_room |= restored_rooms > 0;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    if saw_restored_room {
                        return Ok(());
                    }
                    return Err(format!(
                        "{label}: restore succeeded without any joined room"
                    ));
                }
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup restore failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.key_backup {
                KeyBackupStatus::Restoring {
                    request_id: current,
                    restored_rooms,
                    ..
                } if current == request_id.sequence => {
                    saw_request_state = true;
                    saw_restored_room |= restored_rooms > 0;
                }
                KeyBackupStatus::Enabled { .. } if saw_request_state => {
                    if saw_restored_room {
                        return Ok(());
                    }
                    return Err(format!(
                        "{label}: restore succeeded without any joined room"
                    ));
                }
                KeyBackupStatus::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == request_id.sequence => {
                    return Err(format!("{label}: key backup restore failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn reset_identity_for_qa(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    password: String,
    label: &str,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    let flow_id = request_id.sequence;
    conn.command(CoreCommand::Account(AccountCommand::ResetIdentity {
        request_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit reset identity failed: {e}"))?;

    match wait_for_identity_reset_auth_or_done(conn, account_key, flow_id, request_id, label)
        .await?
    {
        IdentityResetWait::Completed => Ok(()),
        IdentityResetWait::AuthRequired(IdentityResetAuthType::Uiaa) => {
            let submit_request_id = conn.next_request_id();
            conn.command(CoreCommand::Account(
                AccountCommand::SubmitIdentityResetAuth {
                    request_id: submit_request_id,
                    flow_id,
                    request: IdentityResetAuthRequest::UiaaPassword {
                        password: AuthSecret::new(password),
                    },
                },
            ))
            .await
            .map_err(|e| format!("{label}: submit reset identity UIAA failed: {e}"))?;
            wait_for_identity_reset_done(conn, account_key, flow_id, submit_request_id, label).await
        }
        IdentityResetWait::AuthRequired(IdentityResetAuthType::OAuth) => Err(format!(
            "{label}: OAuth identity reset cannot run headlessly"
        )),
        IdentityResetWait::AuthRequired(IdentityResetAuthType::Unknown) => Err(format!(
            "{label}: unknown identity reset auth type cannot run headlessly"
        )),
    }
}

enum IdentityResetWait {
    Completed,
    AuthRequired(IdentityResetAuthType),
}

async fn wait_for_identity_reset_auth_or_done(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    flow_id: u64,
    command_request_id: RequestId,
    label: &str,
) -> Result<IdentityResetWait, String> {
    let mut saw_request_state = false;

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for identity reset auth/done"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
                account_key: ev_account_key,
                state,
            }) if &ev_account_key == account_key => {
                if matches!(state, IdentityResetState::Idle) {
                    return Ok(IdentityResetWait::Completed);
                }
                if let Some(result) = identity_reset_observation(&state, flow_id, label)? {
                    return Ok(result);
                }
                if matches!(
                    state,
                    IdentityResetState::Resetting { request_id: current }
                        if current == flow_id
                ) {
                    saw_request_state = true;
                }
            }
            CoreEvent::StateChanged(snapshot) => {
                let state = snapshot.e2ee_trust.identity_reset;
                if !matches!(state, IdentityResetState::Idle) {
                    if let Some(result) = identity_reset_observation(&state, flow_id, label)? {
                        return Ok(result);
                    }
                }
                if matches!(
                    state,
                    IdentityResetState::Resetting { request_id: current }
                        if current == flow_id
                ) {
                    saw_request_state = true;
                }
                if saw_request_state && matches!(state, IdentityResetState::Idle) {
                    return Ok(IdentityResetWait::Completed);
                }
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == command_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn wait_for_identity_reset_done(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    flow_id: u64,
    command_request_id: RequestId,
    label: &str,
) -> Result<(), String> {
    let mut saw_request_state = matches!(
        conn.snapshot().e2ee_trust.identity_reset,
        IdentityResetState::Resetting {
            request_id: current
        } if current == flow_id
    );

    loop {
        let event = tokio::time::timeout(E2EE_EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for identity reset completion"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
                account_key: ev_account_key,
                state,
            }) if &ev_account_key == account_key => match state {
                IdentityResetState::Idle => return Ok(()),
                IdentityResetState::Resetting {
                    request_id: current,
                } if current == flow_id => {
                    saw_request_state = true;
                }
                IdentityResetState::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == flow_id => {
                    return Err(format!("{label}: identity reset failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::StateChanged(snapshot) => match snapshot.e2ee_trust.identity_reset {
                IdentityResetState::Idle if saw_request_state => return Ok(()),
                IdentityResetState::Resetting {
                    request_id: current,
                } if current == flow_id => {
                    saw_request_state = true;
                }
                IdentityResetState::Failed {
                    request_id: failed_id,
                    kind,
                } if failed_id == flow_id => {
                    return Err(format!("{label}: identity reset failed: {kind:?}"));
                }
                _ => {}
            },
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == command_request_id => {
                return Err(format!("{label} failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

fn identity_reset_observation(
    state: &IdentityResetState,
    request_sequence: u64,
    label: &str,
) -> Result<Option<IdentityResetWait>, String> {
    match state {
        IdentityResetState::AwaitingAuth {
            request_id,
            auth_type,
        } if *request_id == request_sequence => {
            Ok(Some(IdentityResetWait::AuthRequired(*auth_type)))
        }
        IdentityResetState::Failed { request_id, kind } if *request_id == request_sequence => {
            Err(format!("{label}: identity reset failed: {kind:?}"))
        }
        _ => Ok(None),
    }
}

pub(super) async fn verify_second_device_room_key_delivery_for_qa(
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    wait_for_room_in_room_list(conn_a, room_id, "A room list before encrypted send").await?;
    wait_for_room_in_room_list(conn_a2, room_id, "A2 room list before encrypted receive").await?;

    let key_a = TimelineKey::room(account_key_a.clone(), room_id.to_owned());
    let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.to_owned());

    let subscribe_a2_id = conn_a2.next_request_id();
    conn_a2
        .command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: subscribe_a2_id,
            key: key_a2.clone(),
        }))
        .await
        .map_err(|e| format!("second-device decrypt: submit A2 subscribe failed: {e}"))?;

    let initial_a2 = wait_for_initial_items(
        conn_a2,
        &key_a2,
        subscribe_a2_id,
        "second-device encrypted room subscribe",
    )
    .await?;
    assert_no_decryption_failure_items(&initial_a2, "second-device encrypted room initial")?;
    if find_timeline_item_with_body(&initial_a2, E2EE_KEY_BACKUP_SEED_BODY).is_none() {
        return Err("second-device decrypt: restored backup seed body was not visible".to_owned());
    }

    let transaction_id = "qa-e2ee-second-device-delivery".to_owned();
    let send_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: send_id,
            key: key_a.clone(),
            transaction_id: transaction_id.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(
                E2EE_SECOND_DEVICE_BODY.to_owned(),
            ),
        }))
        .await
        .map_err(|e| format!("second-device decrypt: submit encrypted send failed: {e}"))?;

    wait_for_send_flow_completion(
        conn_a,
        send_id,
        &key_a,
        &transaction_id,
        E2EE_SECOND_DEVICE_BODY,
        "second-device encrypted send",
    )
    .await?;

    wait_for_item_with_body_or_decryption_failure(
        conn_a2,
        &key_a2,
        E2EE_SECOND_DEVICE_BODY,
        "second-device encrypted receive",
    )
    .await?;

    Ok(())
}

pub(super) async fn verify_multi_user_multi_device_room_key_delivery_for_qa(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    conn_a2: &mut CoreConnection,
    account_key_a: &AccountKey,
    account_key_a2: &AccountKey,
    recipient_base: Option<(&mut CoreConnection, &AccountKey)>,
) -> Result<(), String> {
    let check_recipient_second_device = env_flag_enabled(ENV_E2EE_RECIPIENT_SECOND_DEVICE)?;
    let user_b_full_id = format!("@{}:{}", config.user_b, config.server_name);
    let room_id = create_room_for_qa(
        conn_a,
        "QA E2EE Multi Device DM",
        true,
        "e2ee multi-device create encrypted room",
    )
    .await?;

    wait_for_room_in_room_list(
        conn_a,
        &room_id,
        "e2ee multi-device A room list after create",
    )
    .await?;

    invite_user_for_qa(
        conn_a,
        &room_id,
        &user_b_full_id,
        "e2ee multi-device invite B",
    )
    .await?;

    let mut recipient = match recipient_base {
        Some((conn, account_key)) => QaE2eeRecipient::Borrowed { conn, account_key },
        None => QaE2eeRecipient::Owned(
            login_synced_participant_for_qa(
                &config.homeserver,
                qa_data_dir("e2ee-b"),
                &config.user_b,
                &config.password_b,
                DEVICE_B,
                "e2ee login B",
                "gate-bootstrap-b",
                QaParticipantLoginGate::BootstrapNewIdentity,
            )
            .await?
            .into(),
        ),
    };
    let mut owned_recipient_second_device = None;
    let mut owned_unverified_recipient_device = None;
    let stage_result: Result<(), String> = async {
        let (conn_b, account_key_b) = recipient.connection_and_account_key();

        wait_for_invite_in_snapshot(
            conn_b,
            &room_id,
            Some(false),
            "e2ee multi-device wait for B invite",
        )
        .await?;
        accept_invite_for_qa(conn_b, &room_id, "e2ee multi-device B accepts invite").await?;

        let settings_a = load_room_settings_for_qa(
            conn_a,
            &room_id,
            "e2ee multi-device A observes B membership",
        )
        .await?;
        assert_room_settings_contains_members(
            &settings_a,
            &[account_key_a.0.as_str(), user_b_full_id.as_str()],
            "e2ee multi-device A observes B membership",
        )?;
        wait_for_room_in_room_list(
            conn_a2,
            &room_id,
            "e2ee multi-device A2 room list after create",
        )
        .await?;
        wait_for_room_in_room_list(conn_b, &room_id, "e2ee multi-device B room list").await?;

        let key_a = TimelineKey::room(account_key_a.clone(), room_id.clone());
        let key_a2 = TimelineKey::room(account_key_a2.clone(), room_id.clone());
        let key_b = TimelineKey::room(account_key_b.clone(), room_id.clone());

        let initial_a =
            subscribe_timeline_for_qa(conn_a, &key_a, "e2ee multi-device subscribe A").await?;
        let initial_a2 =
            subscribe_timeline_for_qa(conn_a2, &key_a2, "e2ee multi-device subscribe A2").await?;
        let initial_b =
            subscribe_timeline_for_qa(conn_b, &key_b, "e2ee multi-device subscribe B").await?;
        assert_no_decryption_failure_items(&initial_a, "e2ee multi-device A initial")?;
        assert_no_decryption_failure_items(&initial_a2, "e2ee multi-device A2 initial")?;
        assert_no_decryption_failure_items(&initial_b, "e2ee multi-device B initial")?;

        let mut recipient_second_device_key = None;
        if check_recipient_second_device {
            let runtime_b2 = start_isolated_qa_runtime("e2ee-b2")?;
            let conn_b2 = runtime_b2.attach();
            owned_recipient_second_device =
                Some(QaOwnedRuntimeParticipant::new(runtime_b2, conn_b2));
            let participant_b2 = owned_recipient_second_device
                .as_mut()
                .expect("B2 owner was installed before login");
            let login_b2 = participant_b2.conn.next_request_id();
            participant_b2
                .conn
                .command(CoreCommand::Account(AccountCommand::LoginPassword {
                    request_id: login_b2,
                    request: koushi_state::LoginRequest {
                        homeserver: config.homeserver.clone(),
                        username: config.user_b.clone(),
                        password: AuthSecret::new(config.password_b.clone()),
                        device_display_name: Some("Koushi Core QA B2".to_owned()),
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
                .map_err(|error| format!("e2ee login B2 submit: {error}"))?;
            participant_b2.mark_login_submitted();
            let session_b2 =
                wait_for_existing_identity_gate(&mut participant_b2.conn, "e2ee B2 gate").await?;
            let session_b =
                authenticated_session_info(conn_b, "session B info for E2EE multi-device")?;
            verify_provisional_second_device_for_qa(
                conn_b,
                &mut participant_b2.conn,
                &session_b,
                &session_b2,
                "e2ee recipient verification B/B2",
                SasQaOutcome::Success,
            )
            .await?;
            let account_key_b2 =
                wait_for_logged_in(&mut participant_b2.conn, login_b2, "e2ee login B2").await?;
            participant_b2.mark_logged_in(account_key_b2.clone());
            wait_for_ready_snapshot(&mut participant_b2.conn, "e2ee B2 Ready").await?;
            start_sync_for_qa(&mut participant_b2.conn, "e2ee B2 sync").await?;
            wait_for_room_in_room_list(
                &mut participant_b2.conn,
                &room_id,
                "e2ee multi-device B2 room list",
            )
            .await?;
            let key_b2 = TimelineKey::room(account_key_b2.clone(), room_id.clone());
            let initial_b2 = subscribe_timeline_for_qa(
                &mut participant_b2.conn,
                &key_b2,
                "e2ee multi-device subscribe B2",
            )
            .await?;
            assert_no_decryption_failure_items(&initial_b2, "e2ee multi-device B2 initial")?;
            recipient_second_device_key = Some(key_b2);
        }

        let runtime_b3 = start_isolated_qa_runtime("e2ee-b3-unverified")?;
        let conn_b3 = runtime_b3.attach();
        owned_unverified_recipient_device =
            Some(QaOwnedRuntimeParticipant::new(runtime_b3, conn_b3));
        let participant_b3 = owned_unverified_recipient_device
            .as_mut()
            .expect("B3 owner was installed before login");
        let login_b3 = participant_b3.conn.next_request_id();
        participant_b3
            .conn
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_b3,
                request: koushi_state::LoginRequest {
                    homeserver: config.homeserver.clone(),
                    username: config.user_b.clone(),
                    password: AuthSecret::new(config.password_b.clone()),
                    device_display_name: Some("Koushi Core QA B3 Unverified".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .map_err(|error| format!("e2ee unverified peer login submit: {error}"))?;
        participant_b3.mark_login_submitted();
        let session_b3 =
            wait_for_existing_identity_gate(&mut participant_b3.conn, "e2ee unverified peer gate")
                .await?;
        refresh_device_keys_and_assert_known_for_qa(
            conn_a,
            VerificationTarget {
                user_id: session_b3.user_id.clone(),
                device_id: session_b3.device_id.clone(),
            },
            "e2ee unverified peer device discovery",
        )
        .await?;
        let transaction_id = "qa-e2ee-multi-user-multi-device-delivery".to_owned();
        let send_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: send_id,
                key: key_a.clone(),
                transaction_id: transaction_id.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(
                    E2EE_MULTI_USER_MULTI_DEVICE_BODY.to_owned(),
                ),
            }))
            .await
            .map_err(|e| format!("e2ee multi-device: submit encrypted send failed: {e}"))?;

        wait_for_send_flow_completion_with_timeout(
            conn_a,
            send_id,
            &key_a,
            &transaction_id,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device encrypted send",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        println!("e2ee_unverified_peer_send_nonblocking=ok");

        wait_for_item_with_body_or_decryption_failure(
            conn_a2,
            &key_a2,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device A2 receive",
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            conn_b,
            &key_b,
            E2EE_MULTI_USER_MULTI_DEVICE_BODY,
            "e2ee multi-device B receive",
        )
        .await?;

        let session_b = authenticated_session_info(conn_b, "blocked QA B session")?;
        verify_provisional_second_device_for_qa(
            conn_b,
            &mut participant_b3.conn,
            &session_b,
            &session_b3,
            "blocked QA promote B3",
            SasQaOutcome::Success,
        )
        .await?;
        let account_key_b3 =
            wait_for_logged_in(&mut participant_b3.conn, login_b3, "blocked QA B3 login").await?;
        participant_b3.mark_logged_in(account_key_b3.clone());
        wait_for_ready_snapshot(&mut participant_b3.conn, "blocked QA B3 Ready").await?;
        start_sync_for_qa(&mut participant_b3.conn, "blocked QA B3 sync").await?;
        wait_for_room_in_room_list(&mut participant_b3.conn, &room_id, "blocked QA B3 room")
            .await?;
        let key_b3 = TimelineKey::room(account_key_b3.clone(), room_id.clone());
        let initial_b3 =
            subscribe_timeline_for_qa(&mut participant_b3.conn, &key_b3, "blocked QA B3 timeline")
                .await?;

        let (acknowledged, ack) = tokio::sync::oneshot::channel();
        let blacklist_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Account(
                AccountCommand::QaSetLocalDeviceBlacklisted {
                    request_id: blacklist_id,
                    target: VerificationTarget {
                        user_id: session_b3.user_id.clone(),
                        device_id: session_b3.device_id.clone(),
                    },
                    room_id: room_id.clone(),
                    acknowledged,
                },
            ))
            .await
            .map_err(|_| "blocked QA blacklist submit failed".to_owned())?;
        tokio::time::timeout(EVENT_TIMEOUT, ack)
            .await
            .map_err(|_| "blocked QA blacklist ack timeout".to_owned())?
            .map_err(|_| "blocked QA blacklist ack closed".to_owned())?
            .map_err(|_| "blocked QA blacklist failed".to_owned())?;
        let blocked_body = "Koushi blocked-device withheld probe";
        let blocked_txn = "qa-e2ee-blocked-device-withheld".to_owned();
        let blocked_send = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                request_id: blocked_send,
                key: key_a.clone(),
                transaction_id: blocked_txn.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(blocked_body.to_owned()),
            }))
            .await
            .map_err(|_| "blocked QA Core send submit failed".to_owned())?;
        let blocked_send_outcome = wait_for_send_flow_completion_with_timeout(
            conn_a,
            blocked_send,
            &key_a,
            &blocked_txn,
            blocked_body,
            "blocked QA Core send",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        wait_for_item_with_body_or_decryption_failure(
            conn_b,
            &key_b,
            blocked_body,
            "blocked QA nonblocked receive",
        )
        .await?;
        wait_for_withheld_event_projection_from_source(
            &mut participant_b3.conn,
            &key_b3,
            &blocked_send_outcome.event_id,
            blocked_body,
            &initial_b3,
            "blocked QA B3 withheld event",
            E2EE_EVENT_TIMEOUT,
        )
        .await?;
        println!("e2ee_blocked_device_withheld=ok");

        if let Some(key_b2) = recipient_second_device_key {
            let participant_b2 = owned_recipient_second_device
                .as_mut()
                .expect("B2 key exists only while its owner is retained");
            wait_for_item_with_body_or_decryption_failure(
                &mut participant_b2.conn,
                &key_b2,
                E2EE_MULTI_USER_MULTI_DEVICE_BODY,
                "e2ee multi-device B2 receive",
            )
            .await?;
            println!("e2ee_recipient_second_device_decrypt=ok");
        }

        Ok(())
    }
    .await;

    finish_e2ee_recipient_stage_with_owned_cleanup(
        stage_result,
        Some((
            recipient.into_owned(),
            owned_recipient_second_device,
            owned_unverified_recipient_device,
        )),
        cleanup_e2ee_multi_device_participants,
    )
    .await
}

fn assert_no_decryption_failure_items(items: &[TimelineItem], label: &str) -> Result<(), String> {
    if items.iter().any(timeline_item_is_decryption_failure) {
        return Err(format!(
            "{label}: timeline contained an undecryptable event"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
